use super::{acquire_with_inputs, data::SourceData, Context, Resolved};
use crate::{runtime::Cancellation, Result, MAX_BYTES};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Condvar, Mutex,
    },
    time::{Duration, Instant},
};

struct FlightState {
    output: Option<(Instant, Result<Arc<SourceData>>)>,
    waiters: usize,
}
struct Flight {
    state: Mutex<FlightState>,
    wake: Condvar,
    cancel: Arc<Cancellation>,
    revision: u64,
}
#[derive(Default)]
pub struct SourceCache {
    entries: Mutex<HashMap<String, Arc<Flight>>>,
    sequence: AtomicU64,
}
struct Waiter(Arc<Flight>);
#[derive(Default)]
struct Budget {
    revisions: std::collections::HashSet<String>,
    bytes: u64,
    completed: HashMap<String, (Arc<SourceData>, String)>,
}
impl Drop for Waiter {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        state.waiters -= 1;
        if state.waiters == 0 && state.output.is_none() {
            self.0.cancel.cancel();
            // Keep the host file-access lease alive until the last producer has
            // released its file/query handles, even when the caller cancels.
            while state.output.is_none() {
                state = self.0.wake.wait(state).unwrap_or_else(|e| e.into_inner());
            }
        }
    }
}
impl SourceCache {
    pub fn load(
        &self,
        source: Resolved,
        ctx: &Context,
        cancel: &Cancellation,
    ) -> Result<(Arc<SourceData>, String)> {
        self.load_graph(source, ctx, cancel, &mut Budget::default())
    }
    fn load_graph(
        &self,
        mut source: Resolved,
        ctx: &Context,
        cancel: &Cancellation,
        budget: &mut Budget,
    ) -> Result<(Arc<SourceData>, String)> {
        cancel.check()?;
        // Keep one snapshot per node throughout this traversal, even when a
        // slow sibling outlives the shared cache TTL or capacity eviction.
        fn without_display(source: &mut Resolved) {
            source.spec.display = None;
            for source in source.upstream.values_mut() {
                without_display(source);
            }
        }
        let mut identity = source.clone();
        without_display(&mut identity);
        let traversal_key = crate::runtime::cache_key(&identity)?;
        if let Some(result) = budget.completed.get(&traversal_key) {
            return Ok(result.clone());
        }
        let mut inputs = std::collections::BTreeMap::new();
        let mut items = None;
        let foreach = source
            .spec
            .provider
            .get("foreach")
            .and_then(|f| f.get("source"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let mut revisions = std::collections::BTreeMap::new();
        let referenced = facetra_contract::template::source_references(&source.spec)?;
        for (name, upstream) in std::mem::take(&mut source.upstream) {
            let (data, revision) = self.load_graph(upstream, ctx, cancel, budget)?;
            if referenced.contains(&name) {
                inputs.insert(name.clone(), data.template_value(cancel)?);
            }
            if foreach.as_ref() == Some(&name) {
                let records = data.records_value(cancel)?;
                items = Some(
                    serde_json::from_value(records)
                        .map_err(|_| "HTTP foreach requires upstream records")?,
                );
            }
            revisions.insert(name, revision);
        }
        cancel.check()?;
        // Presentation never changes acquisition identity. Credential values are
        // transient; the host supplies an opaque credential/permission revision.
        let mut identity = source.clone();
        identity.spec.display = None;
        let file = source.spec.file.as_ref().and_then(|f| ctx.files.get(f));
        let mut local_revision = Vec::new();
        let path = if file.is_some() {
            None
        } else if let Some(path) = &source.spec.file {
            Some(path.clone())
        } else {
            source
                .spec
                .provider
                .get("file_path")
                .and_then(serde_json::Value::as_str)
                .map(|p| format!("data/{p}"))
        };
        if let Some(path) = path {
            local_revision.push(crate::runtime::source_file_revision(
                &ctx.directory,
                &path,
                cancel,
            )?);
            let wal = format!("{path}-wal");
            if source.spec.source.as_deref() == Some("sqlite")
                && std::path::Path::new(&ctx.directory).join(&wal).exists()
            {
                local_revision.push(crate::runtime::source_file_revision(
                    &ctx.directory,
                    &wal,
                    cancel,
                )?);
            }
        }
        let key = crate::runtime::cache_key(&(
            &identity,
            &ctx.workspace,
            &ctx.directory,
            &ctx.revision,
            file,
            local_revision,
            revisions,
        ))?;
        let flight = {
            let mut entries = self
                .entries
                .lock()
                .map_err(|_| "Source cache lock poisoned")?;
            let mut weight = 0;
            entries.retain(|_, entry| {
                let state = entry.state.lock().unwrap_or_else(|e| e.into_inner());
                let bytes = state
                    .output
                    .as_ref()
                    .and_then(|(_, r)| r.as_ref().ok())
                    .map_or(0, |d| d.weight());
                weight += bytes;
                state.waiters > 0 || weight as u64 <= MAX_BYTES
            });
            let existing = entries.get(&key).and_then(|entry| {
                let mut state = entry.state.lock().unwrap_or_else(|e| e.into_inner());
                let expired = state.output.as_ref().is_some_and(|(at, result)| {
                    result.is_err()
                        || (source.spec.source.as_deref() != Some("duckdb")
                            && at.elapsed() >= Duration::from_secs(60))
                });
                if expired || entry.cancel.check().is_err() {
                    return None;
                }
                state.waiters += 1;
                Some(entry.clone())
            });
            if let Some(entry) = existing {
                entry
            } else {
                if entries.len() >= 32 {
                    entries.retain(|_, e| {
                        e.state.lock().unwrap_or_else(|e| e.into_inner()).waiters > 0
                    });
                }
                let entry = Arc::new(Flight {
                    state: Mutex::new(FlightState {
                        output: None,
                        waiters: 1,
                    }),
                    wake: Condvar::new(),
                    cancel: Arc::new(Cancellation::default()),
                    revision: self.sequence.fetch_add(1, Ordering::Relaxed) + 1,
                });
                entries.insert(key.clone(), entry.clone());
                let worker = entry.clone();
                let context = ctx.clone();
                std::thread::spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        acquire_with_inputs(source, context, worker.cancel.clone(), inputs, items)
                    }))
                    .unwrap_or_else(|_| Err("Source execution failed".into()))
                    .map(Arc::new);
                    worker
                        .state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .output = Some((Instant::now(), result));
                    worker.wake.notify_all();
                });
                entry
            }
        };
        let _waiter = Waiter(flight.clone());
        let mut state = flight
            .state
            .lock()
            .map_err(|_| "Source cache lock poisoned")?;
        loop {
            cancel.check()?;
            if let Some((_, result)) = &state.output {
                let data = result.clone()?;
                let revision = format!("{key}:{}", flight.revision);
                if budget.revisions.insert(revision.clone()) {
                    budget.bytes = budget.bytes.saturating_add(data.weight() as u64);
                    if budget.bytes > MAX_BYTES {
                        return Err("Source dependencies exceed 100 MiB".into());
                    }
                }
                budget
                    .completed
                    .insert(traversal_key, (data.clone(), revision.clone()));
                return Ok((data, revision));
            }
            state = flight
                .wake
                .wait_timeout(state, Duration::from_millis(10))
                .map_err(|_| "Source wait failed")?
                .0;
        }
    }
}

#[cfg(test)]
#[path = "tests/cache.rs"]
mod tests;
