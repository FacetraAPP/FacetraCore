# Source and dataset ownership

The application uses ABI v1 `execute`: Rust acquires all declared sources,
imports them into DuckDB, binds dataset parameters and returns a typed frame or
the complete final renderer payload. Swift handles file authorization, Keychain
and UI tasks, then decodes and displays JSON or a value card. No Swift
SQL/network provider or chart-data mapper executes. HealthKit and its examples,
permissions and Apple dependencies have been removed. Existing HealthKit YAML
fails with an explicit message.

| Stage | Owner | Implementation |
| --- | --- | --- |
| Source definitions and references | Rust | `spec.rs`, `sources/mod.rs` |
| HTTP templates, HTTPS, redirects, limits | Rust | `sources/template.rs`, `sources/http.rs`, reqwest |
| Pagination state, foreach, acquisition budgets/retry | Rust | `sources/http.rs` |
| HTML/CSS and XML/XPath extraction | Rust | `sources/extract.rs` |
| Bounded pure HTTP expressions | facetra-contract | `expression.rs`, strict Rhai expressions |
| PostgreSQL / MySQL parameters and reads | Rust | `sources/database.rs`, tokio-postgres / mysql_async |
| SQLite reads / WAL-aware import backup | Rust | `sources/database.rs`, rusqlite |
| CSV reads / JSON conversion / previews | Rust | `sources/data.rs`, directory-relative CSV reader |
| Shared acquisition, TTL, cancellation | Rust | `sources/cache.rs` |
| Dataset import, SELECT/WITH, result cache | Rust | `runtime.rs`, `engine.rs`, bundled DuckDB 1.4.4 |
| Platform file access and credentials | Swift | scoped workspace access, ConnectionStore |
| Final ECharts/HTML/Vega-Lite/Vela/value payload | Rust | `render.rs`, `echarts.rs`, `vela.rs`, shared by app and standalone pipeline |
| Template inspection and validation | facetra-contract | one scanner used by Swift adapters and Rust execution |
| Payload decoding and presentation | Swift | JSON decode, WebKit, value card |

## ABI requests

`execute` contains `sources` (alias → `{spec, params, number_params}`), `context`,
optional dataset `sql`, `params`, and `number_params`. Without `render`, the response
contains `frame`, `row_count`, `snapshot_reused`, `result_reused`. Optional
`render: {chart, params, catalog, locale}` compiles the selected renderer and
returns `payload_json` instead of `frame`, plus the same row count and reuse flags.
`payload_json` is the final client-neutral payload, bounded to 100 MiB. The latest rendered
payload is cached separately from SQL results, so changing style or locale
does not refetch sources or rerun SQL. `source` acquires one source and either returns a frame
or a `df` / `json` / `text` preview. `decode_source`, `compile_source_sql`,
`read_source_bytes`, `sqlite_snapshot` and `validate_source` support host import,
validation and preview tools using the same Rust implementations.

`source_page` accepts `source`, `context`, `offset`, `limit` (1–1024), and an
optional `revision`. The host requests 256 rows initially. The response contains
`frame`, `total_rows`, `offset`, `next_offset` and an opaque `revision` lease.
Subsequent pages pass that lease and read the same frozen snapshot even if its
file changes. A separate initial request obtains an independent lease.
`release_preview {revision}` releases it when the view closes or refreshes;
canceled/discarded initial replies also release their leases. Expired leases
return an error rather than mixing rows from different snapshots.

Context identifies workspace, directory, refresh/credential revision and
optional imported CSV snapshots. `connections` contains only referenced
connection fields and validated provider options, never the entire Keychain.
Additional HTTP PEM roots and SQLite timeout are host capabilities, not YAML.
Raw credentials do not enter dataset SQL or retained cache keys. The host hashes
only referenced values, resolved binding IDs and connection query versions;
unrelated connection edits and display-name edits preserve query reuse.
Source errors avoid response
bodies, connection URLs and parameter values. The host keeps file access alive
through cancellation until the last source producer releases its handles.

## Data contract

Frames contain ordered unique `columns: [{name, type}]` and equal-width `rows`.
Types are string, integer (signed 64-bit), number, boolean, date, timestamp and
JSON text. Null is distinct from empty string/zero. CSV columns remain strings
until authored SQL casts them. Large unsigned integers and arbitrary precision
database decimals remain strings. JSON nested values use the JSON type; remote
SQL JSON/date/time values use lossless text for compatibility. PostgreSQL timestamps
retain UTC ISO 8601 text (microsecond precision), UUIDs use canonical uppercase
text and money retains the existing two-decimal representation. SQLite declared
numeric types survive empty and all-NULL results. Binary SQL values use base64.

The current boundary is bounded JSON, not Arrow or zero-copy. All renderers
receive final payload JSON directly; typed frames are retained for data tables
and diagnostics. Cache
keys stream serialized inputs into SHA-256 rather than retaining full input JSON.
CSV/SQLite revisions come from opened file descriptors (including SQLite WAL
metadata); changed or removed files cannot reuse an old cached frame. Raw
HTTP previews retain records outside recordsPath. Conversion failure does not
prevent inspecting raw text. Shared cache identity excludes display; SQL-only
parameters do not invalidate HTTP/CSV acquisition. Source expiry changes the
DuckDB snapshot revision as well as the provider cache.

## Limits and lifecycle

There are at most 32 sources, a 100 MiB source budget and 100,000 rows per table
or result. HTTP enforces the decoded response size, 30-second timeout and
same-origin HTTPS redirects. Database reads bind parameters and use read-only
transactions; TLS defaults to verify-full, with explicit require/disable modes.
SQLite uses read-only statements, bounded lock waits and a progress handler.
DuckDB external access is disabled after trusted import, with 128 MiB memory,
no spill and no auto-install/load of extensions. Errors discard failed dataset
snapshots; corrected queries can retry. Cancellation is scoped to a caller;
shared producers stop only when their final waiter leaves.

Preview leases retain at most four snapshots per session, with a 100 MiB soft
budget (one already-valid oversize source can remain). The UI appends rows only
after validating lease, offset, column schema and total count. Paging bounds
the initial transport and UI decode; it does not stream source acquisition or
remove the existing 100,000-row limit.

`query` and raw-YAML `facetra_pipeline_run` are test and integration surfaces.
`render_frame` and `validate_render` run the same canonical renderer compiler
for host validation and typed-frame callers. `render_frame` returns
`payload_json`, `row_count`, `snapshot_reused` and `result_reused`. The old
`echarts` / `validate_echarts` session operations are removed. Future Arrow
batch transport should preserve the frame contract and be justified by
measurements; it is not required to add another Rust provider.

## Adding a provider

Implement spec validation, scoped acquisition, conversion and bounded
cancellation in `sources/`. Supply tests for NULL/types/order, parameter
binding, failure/retry, TLS, byte/row limits and source → dataset execution.
Run `Tools/check-rust-coverage.sh`; new providers must meet the source coverage
gate. Swift only needs provider configuration/import UI and rendering support
when the data semantics require it.

### Source DAG projection

Resolved source objects may additionally contain `upstream` (local dependency name
→ resolved source). Its keys must exactly match `spec.depends_on`. Swift preserves
this projection and gathers credentials/imported files recursively; Rust validates
and acquires prerequisites before the dependent. HTTP source templates read original
JSON (or table records) and retain JSON-body types. Shared-flight keys include each
prerequisite revision, so expiry and local input changes invalidate dependents.
All source/execute/preview entry points share these semantics and cancellation.
See the charting contract for path syntax, graph bounds and failure policy.
