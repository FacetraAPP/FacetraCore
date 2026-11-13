use super::{data::SourceData, extract, template, Context, Resolved};
use crate::{runtime::Cancellation, Result, MAX_BYTES};
pub use facetra_contract::models::HttpSpec as Spec;
use facetra_contract::{expression::Expression, models::HttpLimits, validation::url};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

struct Budget {
    requests: Arc<AtomicUsize>,
    exceeded: Arc<AtomicBool>,
    bytes: usize,
    limits: HttpLimits,
}
impl Budget {
    fn request(&self) -> Result<()> {
        if self.requests.fetch_add(1, Ordering::Relaxed) >= self.limits.max_requests {
            return Err("HTTP acquisition request limit exceeded; result is incomplete".into());
        }
        Ok(())
    }
    fn bytes(&mut self, count: usize) -> Result<()> {
        self.bytes = self.bytes.saturating_add(count);
        if self.bytes > self.limits.max_bytes {
            return Err("HTTP acquisition byte limit exceeded; result is incomplete".into());
        }
        Ok(())
    }
}
pub async fn load(
    source: &Resolved,
    ctx: &Context,
    inputs: &BTreeMap<String, Value>,
    items: Option<&[Value]>,
    cancel: &Cancellation,
) -> Result<SourceData> {
    let spec: Spec = serde_json::from_value(serde_json::to_value(&source.spec.provider).unwrap())
        .map_err(|_| "Invalid HTTP source")?;
    spec.validate()?;
    if let Some(e) = &spec.extract {
        extract::validate(e, spec.format.as_deref().unwrap_or("json"))?;
    }
    let limits = spec.limits.clone().unwrap_or_default();
    tokio::time::timeout(
        Duration::from_millis(limits.timeout_ms),
        collect(source, ctx, inputs, items, cancel, &spec, limits),
    )
    .await
    .map_err(|_| "HTTP acquisition timed out; result is incomplete")?
}
async fn collect(
    source: &Resolved,
    ctx: &Context,
    inputs: &BTreeMap<String, Value>,
    items: Option<&[Value]>,
    cancel: &Cancellation,
    spec: &Spec,
    limits: HttpLimits,
) -> Result<SourceData> {
    let mut budget = Budget {
        requests: Arc::new(AtomicUsize::new(0)),
        exceeded: Arc::new(AtomicBool::new(false)),
        bytes: 0,
        limits,
    };
    let origin = Arc::new(Mutex::new(None::<url::Origin>));
    let client = client(ctx, &budget, origin.clone())?;
    let next = spec
        .pagination
        .as_ref()
        .map(|p| Expression::compile(&p.next))
        .transpose()?;
    let select = spec
        .pagination
        .as_ref()
        .and_then(|p| p.records.as_ref())
        .map(|s| Expression::compile(s))
        .transpose()?;
    let single = [Value::Null];
    let items = if let Some(f) = &spec.foreach {
        let items = items.ok_or("HTTP foreach upstream records unavailable")?;
        if items.len() > f.max_items {
            return Err("HTTP foreach item limit exceeded; result is incomplete".into());
        }
        items
    } else {
        &single
    };
    let mut output = Vec::new();
    let mut output_bytes = 2usize;
    for item in items {
        cancel.check()?;
        let mut locals = BTreeMap::from([("item".into(), item.clone())]);
        let mut page = if let Some(p) = &spec.pagination {
            template::render_value_with_locals(
                &serde_json::to_value(&p.initial).unwrap(),
                &source.params,
                &ctx.connections,
                inputs,
                &locals,
            )?
        } else {
            json!({})
        };
        let mut seen_states = HashSet::new();
        let mut seen_requests = HashSet::new();
        let max_pages = spec.pagination.as_ref().map_or(1, |p| p.max_pages);
        for index in 0..max_pages {
            cancel.check()?;
            if !page.is_object() {
                return Err("HTTP pagination state must be an object or () to stop".into());
            }
            if !seen_states.insert(crate::runtime::cache_key(&page)?) {
                return Err("HTTP pagination repeated a state; result is incomplete".into());
            }
            locals.insert("page".into(), page.clone());
            let request = request(spec, source, ctx, inputs, &locals)?;
            {
                let mut allowed = origin.lock().map_err(|_| "HTTP origin unavailable")?;
                if allowed
                    .as_ref()
                    .is_some_and(|o| o != &request.url().origin())
                {
                    return Err("HTTP iteration cannot change HTTPS origin".into());
                }
                *allowed = Some(request.url().origin());
            }
            let fingerprint = crate::runtime::cache_key(&(
                request.method().as_str(),
                request.url().as_str(),
                request
                    .headers()
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_bytes()))
                    .collect::<Vec<_>>(),
                request.body().and_then(|b| b.as_bytes()),
            ))?;
            if !seen_requests.insert(fingerprint) {
                return Err("HTTP pagination repeated a request; result is incomplete".into());
            }
            let (bytes, mut response) = fetch(&client, request, spec, &mut budget).await?;
            let advanced =
                spec.pagination.is_some() || spec.foreach.is_some() || spec.extract.is_some();
            if !advanced {
                let data = SourceData::raw(
                    bytes,
                    spec.format.clone().unwrap_or_else(|| "json".into()),
                    spec.records_path.clone(),
                )?;
                if budget.limits.max_rows < crate::MAX_ROWS
                    && data.dataframe(cancel)?.rows.len() > budget.limits.max_rows
                {
                    return Err("HTTP acquisition row limit exceeded; result is incomplete".into());
                }
                return Ok(data);
            }
            if let Some(e) = &spec.extract {
                response["extracted"] = extract::context(
                    &bytes,
                    spec.format.as_deref().unwrap(),
                    e,
                    response["url"].as_str().unwrap(),
                    cancel,
                )?;
            }
            let mut variables = BTreeMap::from([
                ("page".into(), page.clone()),
                ("response".into(), response),
                ("item".into(), item.clone()),
            ]);
            let records = if let Some(select) = &select {
                select.evaluate(&variables)?
            } else if let Some(e) = &spec.extract {
                Value::Array(extract::records(
                    &bytes,
                    spec.format.as_deref().unwrap(),
                    e,
                    variables["response"]["url"].as_str().unwrap(),
                    cancel,
                )?)
            } else if bytes.is_empty() && variables["response"]["status"] == 204 {
                Value::Array(vec![])
            } else {
                SourceData::raw(
                    bytes,
                    spec.format.clone().unwrap_or_else(|| "json".into()),
                    spec.records_path.clone(),
                )?
                .records_value(cancel)?
            };
            let rows = records
                .as_array()
                .ok_or("HTTP pagination records expression must return an array")?;
            variables.insert("records".into(), records.clone());
            if output.len().saturating_add(rows.len()) > budget.limits.max_rows {
                return Err("HTTP acquisition row limit exceeded; result is incomplete".into());
            }
            for row in rows {
                let mut row = row.clone();
                if spec.foreach.as_ref().is_some_and(|f| f.include_item) {
                    let object = row
                        .as_object_mut()
                        .ok_or("HTTP include_item requires object records")?;
                    if object.insert("_item".into(), item.clone()).is_some() {
                        return Err("HTTP record conflicts with reserved _item field".into());
                    }
                }
                output_bytes = output_bytes.saturating_add(
                    serde_json::to_vec(&row)
                        .map_err(|_| "Invalid HTTP record")?
                        .len()
                        + 1,
                );
                if output_bytes as u64 > MAX_BYTES || output_bytes > budget.limits.max_bytes {
                    return Err(
                        "HTTP collected records exceed byte limit; result is incomplete".into(),
                    );
                }
                output.push(row);
            }
            let Some(next) = &next else {
                break;
            };
            page = next.evaluate(&variables)?;
            if page.is_null() {
                break;
            }
            if !page.is_object() {
                return Err("HTTP next expression must return an object or () to stop".into());
            }
            if index + 1 == max_pages {
                return Err("HTTP pagination page limit exceeded; result is incomplete".into());
            }
        }
    }
    let data = SourceData::raw(
        serde_json::to_vec(&output).map_err(|_| "Invalid HTTP collected records")?,
        "json".into(),
        vec![],
    )?;
    data.records_value(cancel)?;
    Ok(data)
}
fn client(
    ctx: &Context,
    budget: &Budget,
    origin: Arc<Mutex<Option<url::Origin>>>,
) -> Result<reqwest::Client> {
    let requests = budget.requests.clone();
    let exceeded = budget.exceeded.clone();
    let max = budget.limits.max_requests;
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 10 {
                return attempt.error("Too many redirects");
            }
            if attempt.url().scheme() != "https"
                || origin.lock().ok().and_then(|o| o.clone()).as_ref()
                    != Some(&attempt.url().origin())
            {
                return attempt.stop();
            }
            if requests.fetch_add(1, Ordering::Relaxed) >= max {
                exceeded.store(true, Ordering::Relaxed);
                return attempt.error("Request budget exceeded");
            }
            attempt.follow()
        }));
    for pem in &ctx.http_roots {
        let certificates = reqwest::Certificate::from_pem_bundle(pem.as_bytes())
            .map_err(|_| "Invalid host certificate")?;
        if certificates.is_empty() {
            return Err("Invalid host certificate".into());
        }
        for certificate in certificates {
            builder = builder.add_root_certificate(certificate);
        }
    }
    builder
        .build()
        .map_err(|_| "HTTP transport unavailable".into())
}
fn request(
    spec: &Spec,
    source: &Resolved,
    ctx: &Context,
    inputs: &BTreeMap<String, Value>,
    locals: &BTreeMap<String, Value>,
) -> Result<reqwest::Request> {
    let render = |s: &str| {
        template::render_with_locals(s, &source.params, &ctx.connections, inputs, locals)
            .map(|v| super::data::text(&v))
    };
    let mut target = url(&render(&spec.url)?)?;
    for (key, value) in &spec.params {
        let value = template::render_value_with_locals(
            value,
            &source.params,
            &ctx.connections,
            inputs,
            locals,
        )?;
        if !value.is_null() || spec.pagination.is_none() && spec.foreach.is_none() {
            target
                .query_pairs_mut()
                .append_pair(key, &super::data::text(&value));
        }
    }
    let method = spec.method.as_deref().unwrap_or("get").to_ascii_uppercase();
    let mut request = reqwest::Request::new(
        reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| "Invalid method")?,
        target,
    );
    let accept = match spec.format.as_deref().unwrap_or("json") {
        "csv" => "text/csv",
        "html" => "text/html",
        "xml" => "application/xml, text/xml",
        _ => "application/json",
    };
    request
        .headers_mut()
        .insert("Accept", reqwest::header::HeaderValue::from_static(accept));
    if let Some(body) = &spec.json {
        let body = template::render_value_with_locals(
            body,
            &source.params,
            &ctx.connections,
            inputs,
            locals,
        )?;
        request.headers_mut().insert(
            "Content-Type",
            reqwest::header::HeaderValue::from_static("application/json"),
        );
        *request.body_mut() = Some(
            serde_json::to_vec(&body)
                .map_err(|_| "Invalid JSON body")?
                .into(),
        );
    }
    if let Some(body) = &spec.data {
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        for (key, value) in body {
            let value = template::render_value_with_locals(
                value,
                &source.params,
                &ctx.connections,
                inputs,
                locals,
            )?;
            if !value.is_null() || spec.pagination.is_none() && spec.foreach.is_none() {
                form.append_pair(key, &super::data::text(&value));
            }
        }
        request.headers_mut().insert(
            "Content-Type",
            reqwest::header::HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        *request.body_mut() = Some(form.finish().into());
    }
    for (key, value) in &spec.headers {
        let name = reqwest::header::HeaderName::from_bytes(key.as_bytes())
            .map_err(|_| "Invalid HTTP header")?;
        let mut value = reqwest::header::HeaderValue::from_str(&render(value)?)
            .map_err(|_| "Invalid HTTP header")?;
        value.set_sensitive(true);
        request.headers_mut().insert(name, value);
    }
    Ok(request)
}
async fn fetch(
    client: &reqwest::Client,
    request: reqwest::Request,
    spec: &Spec,
    budget: &mut Budget,
) -> Result<(Vec<u8>, Value)> {
    let attempts = spec.retry.as_ref().map_or(1, |r| r.max_attempts);
    for attempt in 0..attempts {
        budget.request()?;
        let sent = client
            .execute(
                request
                    .try_clone()
                    .ok_or("HTTP request cannot be replayed")?,
            )
            .await;
        if budget.exceeded.load(Ordering::Relaxed) {
            return Err("HTTP acquisition request limit exceeded; result is incomplete".into());
        }
        let mut response = match sent {
            Ok(r) => r,
            Err(_) if attempt + 1 < attempts => {
                tokio::time::sleep(Duration::from_millis(
                    spec.retry
                        .as_ref()
                        .unwrap()
                        .delay_ms
                        .saturating_mul(1 << attempt),
                ))
                .await;
                continue;
            }
            Err(_) => return Err("HTTP request failed".into()),
        };
        let status = response.status();
        let response_url = response.url().to_string();
        let mut headers = BTreeMap::<String, String>::new();
        for (name, value) in response.headers() {
            if name == "set-cookie" {
                continue;
            }
            let value = String::from_utf8_lossy(value.as_bytes());
            headers
                .entry(name.as_str().into())
                .and_modify(|old| {
                    old.push_str(", ");
                    old.push_str(&value);
                })
                .or_insert_with(|| value.into_owned());
        }
        if response
            .content_length()
            .is_some_and(|n| n > budget.limits.max_bytes.saturating_sub(budget.bytes) as u64)
        {
            return Err("HTTP acquisition byte limit exceeded; result is incomplete".into());
        }
        let mut bytes = Vec::new();
        let mut read_failed = false;
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    budget.bytes(chunk.len())?;
                    bytes.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(_) => {
                    read_failed = true;
                    break;
                }
            }
        }
        if read_failed {
            if attempt + 1 < attempts {
                tokio::time::sleep(Duration::from_millis(
                    spec.retry
                        .as_ref()
                        .unwrap()
                        .delay_ms
                        .saturating_mul(1 << attempt),
                ))
                .await;
                continue;
            }
            return Err("HTTP response read failed".into());
        }
        if !status.is_success() {
            if [429, 502, 503, 504].contains(&status.as_u16()) && attempt + 1 < attempts {
                let delay = headers
                    .get("retry-after")
                    .and_then(|s| {
                        s.parse::<u64>()
                            .ok()
                            .map(|n| n.saturating_mul(1000))
                            .or_else(|| {
                                chrono::DateTime::parse_from_rfc2822(s).ok().map(|d| {
                                    (d.with_timezone(&chrono::Utc) - chrono::Utc::now())
                                        .num_milliseconds()
                                        .max(0) as u64
                                })
                            })
                    })
                    .unwrap_or_else(|| {
                        spec.retry
                            .as_ref()
                            .unwrap()
                            .delay_ms
                            .saturating_mul(1 << attempt)
                    });
                tokio::time::sleep(Duration::from_millis(delay)).await;
                continue;
            }
            return Err("HTTP request returned an unsuccessful status".into());
        }
        if spec.pagination.is_none() && spec.foreach.is_none() && spec.extract.is_none() {
            return Ok((bytes, Value::Null));
        }
        let json = if spec.format.as_deref().unwrap_or("json") == "json" {
            match serde_json::from_slice::<Value>(&bytes) {
                Ok(v) => v,
                Err(_)
                    if spec.pagination.is_none() && spec.foreach.is_none()
                        || status.as_u16() == 204 && bytes.is_empty() =>
                {
                    Value::Null
                }
                Err(_) => return Err("Invalid JSON response".into()),
            }
        } else {
            Value::Null
        };
        let links = links(
            headers.get("link").map(String::as_str).unwrap_or(""),
            &response_url,
        )?;
        let text = if matches!(spec.format.as_deref(), Some("html" | "xml" | "csv")) {
            std::str::from_utf8(&bytes)
                .map_err(|_| "HTTP text response must be UTF-8")?
                .to_owned()
        } else {
            String::new()
        };
        return Ok((
            bytes,
            json!({"status":status.as_u16(),"headers":headers,"url":response_url,"json":json,"text":text,"links":links}),
        ));
    }
    Err("HTTP request failed".into())
}
fn split_header(text: &str, delimiter: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut angle, mut quoted, mut escaped, mut start) = (false, false, false, 0);
    for (i, c) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quoted && c == '\\' {
            escaped = true;
            continue;
        }
        match c {
            '"' if !angle => quoted = !quoted,
            '<' if !quoted => angle = true,
            '>' if !quoted => angle = false,
            c if c == delimiter && !angle && !quoted => {
                parts.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}
fn links(text: &str, base: &str) -> Result<BTreeMap<String, String>> {
    let parts = split_header(text, ',');
    let mut result = BTreeMap::new();
    for part in parts {
        let Some((target, attributes)) = part
            .trim()
            .strip_prefix('<')
            .and_then(|s| s.split_once('>'))
        else {
            continue;
        };
        for attribute in split_header(attributes, ';') {
            if let Some((key, value)) = attribute.trim().split_once('=') {
                if key.trim().eq_ignore_ascii_case("rel") {
                    let target = url::Url::parse(base)
                        .and_then(|u| u.join(target))
                        .map_err(|_| "Invalid HTTP Link URL")?
                        .to_string();
                    for rel in value.trim().trim_matches('"').split_whitespace() {
                        result.insert(rel.into(), target.clone());
                    }
                }
            }
        }
    }
    Ok(result)
}
