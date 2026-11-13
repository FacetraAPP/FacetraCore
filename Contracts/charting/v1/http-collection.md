# HTTP collection — pre-release v1 extension

This document is normative alongside specification.md and the generated schemas.
Existing sources without the new settings retain their raw JSON/CSV semantics.
Older development clients must upgrade to read the extension.

## State protocol

An HTTP source may have pagination containing initial (required object), next
(required expression), records (optional expression), and max_pages (default
100, range 1–1000). Each iteration renders the existing request from page,
fetches and selects records, accepts them exactly once, then evaluates next.
An object replaces the entire state. Unit `()` (JSON null) terminates after
accepting this page. Other return types fail. Short/empty pages never imply stop.
Reaching max_pages with a nonterminal state fails as incomplete; terminating
on the last allowed page succeeds. Duplicate states or identical rendered
requests within one item's pagination fail. State may contain compound cursors.

Request templates add page.path (pagination only) and item.path (foreach only),
including whole-object typed JSON substitutions. Initial allows existing
params/connections/sources templates and item, but cannot reference page.
Null query/form substitutions omit fields; explicit JSON null and the literal
string null retain their meanings. Expressions are never interpolated or
rescanned as templates. Upstream references in initial require depends_on and
participate in parameter/credential dependency inspection and source renaming.

Expressions use strict Rhai 1.27 expression syntax: arithmetic, comparisons,
short-circuit logic, indexing, map properties, `.len`, `.contains(key)`,
`if condition { expression } else { expression }` and `#{key: value}` maps.
`()` represents null. No statements, assignments, loops, closures, user
functions, imports/modules, dynamic eval, printing, clocks or I/O are exposed.
The pure function url_join(base, relative) resolves body-provided relative URLs.
The runtime pins the interpreter version; semantics changes require review.

Scopes: page, item (null outside foreach), response, and additionally records
in next. response contains status, headers (lowercase keys; repeated values
joined; Set-Cookie excluded), final url, parsed json for JSON, UTF-8 text for
CSV/HTML/XML, links (Link relation to resolved absolute URL), and extracted
(markup context fields). Missing required paths fail; optional keys require an
explicit contains check. Records expressions must return arrays. Without one,
existing recordsPath/CSV conversion or markup extraction applies. recordsPath
and records expressions are mutually exclusive. A 204 empty response selects
an empty array by default; next still decides termination.

Expressions are compiled at validation and once per acquisition, with source
length 4096 UTF-8 bytes, expression depth 32, 20,000 operations, computed string
size 1 MiB, array length 100,000, map size 1024, serialized input/output 8 MiB,
and output depth 64/one million nodes. Outputs must be finite JSON-compatible
values. Errors are redacted and fail the acquisition.

## Foreach and result semantics

foreach contains source (required direct depends_on name), max_items (default
1000, 1–10,000), include_item (default false). It consumes the upstream's records
after recordsPath/pagination, preserving JSON scalar/nested types before table
conversion. Each item gets a fresh initial state. All items share one budget
and execute sequentially in input order. Empty input performs no child requests.
include_item attaches the original input as `_item` to each output object;
non-object records or existing _item fields fail rather than lose data.

Pagination, foreach and extraction produce one merged JSON record array for
downstream templates and previews. Ordinary HTTP sources retain their original
response before recordsPath. All-or-nothing publication, shared acquisition,
credential revisions, TTL, per-waiter cancellation and frozen previews retain
DAG semantics. Failure/refresh restarts the collection at initial state;
individual pages are not separately cached. Source dependency graphs stay
acyclic; iteration occurs inside acquisition.

## Markup

Formats html/xml require extract; JSON/CSV cannot use extract. Markup must be
UTF-8, at most 8 MiB per response, with no script execution or linked resources.
XML DTDs/entities are rejected. HTML rows use CSS; `.` as a field selector means
the current row. XML rows use XPath 1.0 and must return nodes; field XPath is
relative to each row and may return nodes, attributes or scalars. namespaces
maps prefixes to URIs, including aliases for Atom default namespaces.

Extraction has rows (required selector), fields (required map), optional
context (document-level field map), namespaces (default empty). Field settings:
selector (required), attribute (optional), all (default false), required
(default true), json (default false), resolve_url (default false). Text is
concatenated descendant text with surrounding whitespace trimmed. Ordinary
values are strings; SQL handles type conversion. all returns an array;
otherwise the first value is selected. Missing optional values are null/[];
missing required values fail. json parses embedded JSON/JSON-LD without
executing JavaScript; resolve_url uses the final response URL. Both flags
cannot be true. context appears in response.extracted for pagination metadata.
Selectors are 1–1024 bytes; fields/context each have at most 128 entries;
namespaces at most 32. Invalid selectors, malformed XML/JSON or required-field
failures abort before publishing. Selector syntax is checked before network I/O.

## Acquisition policy

limits are positive: max_requests default 1000/maximum 10,000; max_bytes default
and maximum 104857600; max_rows default and maximum 100000; timeout_ms default
60000/maximum 300000. Request budget counts pages, items, retries and redirects.
max_bytes bounds cumulative decoded bodies including unsuccessful retry bodies,
and separately bounds collected output. Custom row limits also apply to ordinary
single requests. Time spans the entire acquisition, including Retry-After waits.
Upstream acquisitions retain their own budgets; the existing DAG retained-data
budget also applies. Concurrency is one; successful output preserves order.

retry is opt-in: max_attempts default 3/range 1–5, delay_ms default 250/range
0–10000, retry_post default false. GET retries transport errors and
429/502/503/504 with exponential delay; Retry-After seconds or HTTP dates override
delay. POST requires explicit retry_post: true asserting replay safety.
Retries replay the exact request and append a page only after success.
Expression/extraction errors are not retried. The acquisition timeout bounds waits.

All page/item/redirect requests stay on the acquisition's initial HTTPS origin.
Discovered links cannot send credentials to another origin. Expressions cannot
relax URL checks, budgets or cancellation. Errors contain no request URLs,
credentials, response bodies, bound values or authored expression text.

The acquired result is an atomic local snapshot, not a remote transaction.
Changing offset/page datasets can still skip/duplicate records. Prefer stable
ordering, server cursors/snapshot tokens or fixed windows; explicit SQL dedup
cannot recover missing records. Cookie sessions, browser rendering, resumable
checkpoints and historical accumulation are outside this extension.
