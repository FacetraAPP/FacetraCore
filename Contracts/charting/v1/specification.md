# Charting YAML Contract v1

The canonical contract is `facetra-contract` in `contract/`. Its Rust models
and Serde defaults generate the JSON Schema assets in this directory. Structural
cross-field constraints are applied in `contract/src/schema.rs`, used by the
same library for decoding and static validation.
Each kind schema is self-contained so `facetra schema show --kind chart|dashboard`
can be passed directly to a JSON Schema 2020-12 validator without downloading
references. `common.schema.json` accepts either document kind and contains the
shared definitions.

Run `Tools/check-contract.sh --write` after changing models or structural rules;
commit the generated assets. `Tools/check-contract.sh` checks formatting, lints,
schema drift, compatibility samples and the dependency boundary. Core quality
checks run it automatically. `fixtures/cases.json` is the shared expected-result
manifest used by Rust, JSON Schema and Swift tests. It distinguishes structural
validity from static workspace validity and compares diagnostic codes, not text.
Tests also read the current `Docs/examples`, UI fixture workspaces and, when
present, the pinned Discover workspaces. Swift additionally checks actual App
and CLI template output against the exact schemas returned by automation.

## Versions and compatibility

- **Status:** pre-release; v1 may still receive reviewed extensions before the first release. The product fit, freeze decision and remaining host-level
  limits are recorded in [the v1 stability review](stability-review.md).
- **YAML Contract:** `1`, discovered in `doctor.result.contract` and the
  `x-facetra-contract` schema annotation. The supported range is currently `[1]`.
- **Chart documents:** retain the existing unversioned `metadata`, `params`,
  `sources`, `dataset`, `chart` shape. Do not add a `version` field to old charts.
- **Dashboard documents:** retain `version: 1`, `kind: Dashboard`, `metadata`,
  `params`, `tabs`. Old valid files require no rewriting.
- **App/CLI IPC:** `protocolVersion: 1` identifies the automation transport, not
  the YAML format. The application release version is independent of both.

The product has not shipped. Reviewed additions may still update v1, its generated
schemas and the explicit `baseline.json` before the first release. The message
catalog, localized text references and optional parameter `width` are pre-release extensions; older
development clients cannot read the new syntax and must be updated. Existing
string documents keep their meaning and require no rewriting. After the first
release, incompatible typed fields, defaults, discriminators or reference scope
changes require a new major and an explicit migration path while keeping v1 readable.

The schema baseline remains a separate review guard: regeneration alone does not
update it. Update fingerprints deliberately alongside the documented pre-release
change, preserve existing compatibility samples, and run the shared Rust/Swift corpus.

## Localized display text

Chart and Dashboard accept optional `i18n: {defaultLocale, messages}`. `messages`
maps resource-local keys to locale/string maps. `metadata.title`, `metadata.notes`,
parameter labels and Dashboard tab titles accept a fixed string or `{i18n: key}`.
ECharts options also resolve explicit reference objects before rendering; data,
SQL, source templates, scripts and binding results are never recursively localized.
Resolved strings are never interpreted as script expressions.

Every key supplies a nonblank translation for `defaultLocale`. Locale tags match
case-insensitively, cannot be duplicated by case, and use the language/script/region
hyphen syntax specified by the schema. Hosts try the requested language and script/
region, then parent language, then the declared default. Missing keys report
`i18n.missing`; incomplete defaults or ambiguous locales report `i18n.invalid`, with
structural JSON Pointer paths. These are workspace checks; schema validity alone
does not prove that references exist. Each resource owns its catalog, including
Charts embedded in Dashboards. IDs, parameter values and column bindings remain
language independent. See [authoring and runtime details](../../../Docs/localization.md).

## Parsing and defaults

A resource is one YAML mapping with string keys. YAML is resolved to JSON types
before model decoding using YAML 1.2 scalar resolution. Booleans use
`true`/`false`; YAML 1.1 spellings such as `yes`/`no` remain strings. ISO dates
remain strings, including unquoted calendar dates. Quoted `"12"` is a string, `12` and integral `12.0` are
integers, and finite fractional values are numbers. Declared `number` parameters
are restored to floating-point values at the runtime boundary. Non-finite
numbers, non-string mapping keys, custom YAML tags, multiple documents and
repeated mapping keys are rejected. Anchors and merge keys are expanded before
validation; an explicit field can override an inherited merged field.

Unknown fields are rejected in typed objects and renderer/provider variants.
Open maps are limited to named sources, parameter bindings, request fields and
renderer option content. Renderer option semantics and actual column bindings
remain renderer capabilities; a structurally valid option object is not proof
that a renderer can execute it.

Optional fields represented by `Option` accept omitted or null values where
the generated schema allows null. Defaulted non-optional fields may be omitted,
but cannot be null. A null parameter default means no default. An HTTP
`json: null` is an explicit JSON null body, not an omitted body. Renderer-specific
fields remain forbidden in other renderers even when set to null.

| Field | Default when omitted |
| --- | --- |
| `metadata.notes` / `metadata.tags` | empty string / empty list |
| Chart / Dashboard `params` | empty list |
| Chart `sources` | empty mapping |
| Parameter `label` / `required` / `advanced` | parameter name / true / false |
| Parameter `width` | automatic content sizing when omitted or null |
| Source options `value` column | `value` |
| Panel `height` / `params` | 240 / empty mapping |
| Grid `columns` / `row_height` / `gap` / `min_width` | 12 / 40 / 12 / 1200 |
| Grid panel `width` / `row_span` | grid columns / 4 |
| HTTP `method` / response `format` / `display` | get / json / df |

Dates remain strings in `YYYY-MM-DD`, use valid Gregorian calendar dates with
four-digit nonzero years, and do not accept numeric bounds. An optional date
may be the empty string. Integer parameters use signed 64-bit values; number
parameters use finite double-precision values. Numeric bounds are inclusive.
Static option values and defaults must match the declared parameter type, bounds and choices.
Required parameters can be authored without a default; execution then requires
an input or explicit binding.

`chart.renderer` is required and must be one of `value`, `echarts`, `vela`,
`html`, or `vega_lite`. The former implicit `default` renderer and its
`plot / x / y / average / color / x_end` fields are removed. Documents using
those fields or omitting `renderer` are rejected; there is no fallback or
automatic migration.

Parameter `width` is an optional finite positive number in native points, shared
by Chart and Dashboard parameters. It sets the width of the title-above-control
block, overriding content sizing and the automatic width cap. Hosts constrain
it to the available container width on narrow screens. Omission or null keeps
automatic sizing. It has no effect on parameter resolution, SQL or bindings.

## Validation levels

1. **Structure:** a document matches the kind schema. It does not establish
   that references exist or that a client can execute/render the document.
2. **Workspace:** the caller supplies an explicit collection of Chart documents.
   The contract validates IDs, safe relative data paths, parameters and bindings,
   reference scope, missing targets, dependency cycles and Dashboard layout.
   Validation never discovers files, reads credentials or contacts a network.
3. **Execution/capability:** clients authorize directories, coordinate files,
   resolve scoped credentials, acquire data, bind actual columns, render output
   and enforce their supported operations. `doctor.result.capabilities` reports
   the current Mac client's renderers and providers independently of v1 validity.

Current client capability declarations are independent of Contract v1:

| Client | Renderers / sources | Operations |
| --- | --- | --- |
| macOS App | value, ECharts, Vela, HTML, Vega-Lite / CSV, HTTP, database, SQLite | Authorized loading, static validation, execution, editing/saving, interactive rendering; automation and PNG export |
| iOS App | value, ECharts, Vela, HTML, Vega-Lite / CSV, HTTP, database, SQLite | Authorized loading, static validation, execution, editing/saving and interactive rendering; no Mac automation endpoint |
| `facetra` app CLI | Uses the paired Mac App's declared capabilities | Workspace/chart IPC commands; no Discover browsing, import, embedded data engine or renderer |
| `facetra-render` core CLI | value, ECharts, Vela, HTML, Vega-Lite / CSV, SQLite, HTTP, database through the core engine | Read-only workspace inspection, validation, payload compilation and PNG batch rendering; no app CRUD, Keychain or registration |
| `facetra-contract` | Describes all v1 variants without executing them | Structure and static workspace checks only |

The Rust library has no DuckDB, database driver, HTTP client, Keychain or UI
framework dependency. `facetra-pipeline` delegates pure models, parameter handling,
reference resolution, SQL placeholder syntax and template dependency parsing to
it. Swift uses `facetra_contract_run` through the existing Rust string ABI for
parsing, static validation and parameter/reference operations. The production
`facetra` CLI keeps using the paired App and does not link the execution engine.
`facetra-render` is the separate standalone consumer of `facetra-pipeline` and
the renderer assets. There is no Web client or WASM SDK in this version.

## Scope and references

IDs are 1–80 ASCII letters, digits, underscores or hyphens. CSV paths start with
`data/`; SQLite `file_path` is relative to `data/`. Empty, hidden, dot/traversal
segments and backslashes are rejected. Existence, symlinks and directory grants
are host storage concerns.

References use `#/sources/name`, `chart_id.yaml#/sources/name`, or
`chart_id.yaml#/dataset`. A file identifier is a chart metadata ID regardless of
its host folder. References cannot escape a supplied workspace, use directory
segments, or resolve URLs. Internal references inherit owner parameters;
external references receive explicit `with` bindings and target defaults.
Resolving a single source requires only parameters used by its acquisition
templates or SQL, not unrelated required parameters of the exporting chart.
Resolving a whole chart or referenced dataset requires the target's complete
required parameter set. Missing optional parameters remain absent; they are not
converted to null values or implicitly supplied to SQL/templates.
Bindings are scalar literals or exactly `{param: name}`. Parameter option
sources additionally detect dependency on their own parameter through renamed
bindings. Reference recursion is limited to 64 nodes.

Dashboard tab IDs are unique; panel IDs are unique across all tabs. Panels
reference supplied charts and bind compatible parameter types. Required target
parameters without defaults need explicit bindings. Grid dimensions, spans and
compact heights follow the generated per-field ranges; static validation also
checks spans against the owning grid, minimum whole-card height (160 points)
and that grid-only fields are not used on stacked tabs.

## Diagnostics and host-owned state

The JSON contract boundary returns `{ok: value}` or
`{error: {code, path, resource?, line?, column?}}`. `path` is a JSON Pointer; `resource` identifies
a related resource when useful. YAML parser locations are one-based and never
include a source snippet. Codes such as `structure.invalid`,
`parameter.invalid`, `reference.syntax`, `reference.missing`, `reference.cycle`,
`reference.limit` (bounded dependency analysis),
`binding.type`, `binding.required`, and `dashboard.layout` are stable client
keys. Clients localize display text. Diagnostics do not include raw YAML,
parameter values, SQL text, request bodies or credentials.

Editor model decoding is deliberately separate from complete contract validation:
`decode_model` accepts known, typed fields in unfinished providers and layouts;
`decode` requires the published kind schema. Both use definitions generated
from the same models. Swift editors use model decoding to preserve drafts;
save/validate operations always enforce the complete contract.

Shared models contain authored document data only. Host folders, absolute paths,
permissions, window state, revisions and save transactions are excluded. YAML
comments, editor trees and original bytes remain owned by the existing editing
and storage layers; validation does not reserialize files or change revision
hashes. Existing comment, path and optimistic-save regression tests remain in
place.

## Pre-release HTML and Vega-Lite extension

The renderer enum additionally accepts `html` and `vega_lite`. Both require an
object `chart.option` and reject incompatible fields such as `value` and
`chart.scripts`.
HTML options accept only nonempty string `html`, optional string `css`, and
optional object/array `data`; authored JavaScript is not supported. Existing
Dataset, SQL, parameter and reference semantics remain unchanged. Typed data
bindings and i18n substitutions are resolved before rendering. Host behavior is
documented in `Docs/declarative-renderers.md`. This is an explicitly documented
pre-release v1 extension; old clients require an update to read the new renderers.

## Pre-release renderer ownership change

Facetra now prepares the complete renderer payload in Rust for every renderer.
Swift hosts decode and display the returned JSON or value payload; they do not
bind chart series, calculate coordinates, or synthesize renderer templates.
This removes the Swift Charts host and the former `default` renderer entirely.
The generated schemas, fixtures and explicit baseline were updated together.
This is an intentional pre-release breaking change: removed fields are rejected
and no compatibility path is provided.

## Source dependency DAG (pre-release v1 extension)

The subsequent [HTTP collection extension](http-collection.md) adds state-driven
pagination, bounded expressions, foreach, markup extraction and explicit retry.
Its rules supersede the original no-iteration statement below only when these
new optional acquisition settings are present.

Concrete sources accept optional `depends_on: [sourceName, ...]` (omitted or null
means no dependencies). Names are unique local source keys, at most 32, following
the existing ID syntax. Reference aliases keep their existing `ref / with / display`
shape; use a local alias to depend on an external source. A concrete source's
prerequisites resolve in its owning document and parameter scope, including
transitive `with` bindings. Static validation rejects missing nodes, self/indirect
cycles and cycles mixing `ref` with `depends_on`, including unused exports.

HTTP request values in `url`, `headers`, `params`, nested `json` and `data` may use
`{{ sources.name.path }}`. `name` must appear directly in that source's
`depends_on`, even if reachable transitively. Dot segments select object keys or
zero-based array indexes (`sources.items.0.id`); `{{ sources.name }}` selects the
whole result. Segments use the existing ID characters, so keys containing dots,
spaces or other punctuation cannot be addressed with this syntax. No expressions,
bracket indexing, filters on source results or implicit dependency inference.

HTTP JSON references read the original response, before `recordsPath` or display
conversion; CSV/database/SQLite results expose an array of row objects. A whole
JSON-body substitution retains JSON types, including arrays, objects and null.
String interpolation and non-JSON request fields stringify values using the
existing request-template rules. Inserted text is never evaluated as a second
template. Missing paths, out-of-range indexes and invalid upstream JSON are
execution errors; explicit null remains null. Use query `params` for transport
escaping of query values; URL substitution does not add a new URL-encoding filter.
No credentials or response values enter authored YAML or static diagnostics.

Only requested sources and their prerequisites execute. Every declared prerequisite
must succeed before its dependent starts, even when the edge only specifies order.
Unrelated branches are not guaranteed to run concurrently, and dependency-list
order is not a scheduling guarantee. Failure/cancellation stops the dependent;
there is no automatic retry, pagination, item fan-out or partial-result policy.
Shared acquisition preserves existing in-flight reuse and per-caller cancellation.
Downstream cache identity includes prerequisite acquisition revisions. The same
rules apply to datasets, single-source previews and parameter option sources;
option dependency analysis includes transitive upstream parameters.

The existing 64-level reference limit also covers dependency edges. Resolution
and static parameter-dependency analysis each cap expanded traversal at 4096
nodes; excessively dense graphs fail with a bounded error. Runtime enforces a
100 MiB aggregate acquisition budget across unique dependency revisions in a
source load, in addition to existing response/table limits. The internal resolved
projection adds optional `upstream` maps alongside `spec / params / number_params`;
these are host execution data, not additional authored source fields.

See `fixtures/valid/source-dag.yaml` and the corresponding invalid DAG fixtures.
Existing files without dependencies retain their behavior. Old development
clients must upgrade before reading this extension.
