> Historical PR #20 evaluation, before production migration. The application now links Rust; see [current runtime and build instructions](README.md). Original samples and conclusions below are preserved as historical evidence.

# Rust pipeline validation and migration decision

This report covers the opt-in prototype on this branch. The production SwiftUI
application still uses the Swift runtime; it has not been switched to Rust.
Measurements and platform outcomes below must be read with the stated limits.

## Reproduction and environment

Host: Apple M4 Pro, 24 GiB RAM, arm64 macOS 27.0.1 (26A434), Xcode 27.0
(27A266a), SDK 27.0, Rust 1.90.0, Apple Swift 6.4 (swiftlang-6.4.0.34.1). Swift package minimums: macOS 14 / iOS 17;
the application's target settings use 26.6 (project-level settings also contain
27.0). Rust archives explicitly target the shared package's minimums; the smoke
app targets iOS 26.6, without changing the shipping project.

Swift's pinned duckdb-swift revision `2bc1adcf863f83994c27a909d475f8185fbabfa3`
contains DuckDB `v1.6.0-dev11145` (source ID `3305afbcb0`). The default Rust
bundle is `duckdb` / `libduckdb-sys` 1.4.4. Those versions differ.

A **common native engine control** was also built and actually run: relink
Swift release's `Cduckdb.o` as a host dylib, then build Rust with
`--no-default-features` against that same native object implementation. Both
paths report `v1.6.0-dev11145`; native code/extension set/compiler flags are the
same. Rust still owns its connection, CSV tables and queries through its C
binding; no Swift processing runs in Rust's path. The Rust wrapper remains
1.4.4, so this demonstrates compatibility for the tested C APIs/corpus, not
universal future ABI compatibility. The control contains an absolute local
dylib dependency and is **not** the iOS/XCFramework shipping scheme. Default
iOS validation still uses the 1.4.4 bundle. A production package needs one
pinned native engine and extension configuration across platforms.

The previous dylib failure is reproduced by the upstream
[Rust/LLVM strip bug](https://github.com/rust-lang/rust/issues/157750).
Cargo can implicitly strip debuginfo when debug information is disabled.
Explicit `strip = "none"` with `debug = 0` fixes loading on this host while
keeping native debug information out of the cache. `vtool` confirms macOS 14.0
and SDK 27.0; `dlopen` and the JSON C ABI execute successfully. Do not solve it
by lowering deployment targets below the project's supported minimum.

See the README for exact sequential commands. `Tools/benchmark-pipeline.sh`
collects one case/backend per fresh process. Raw samples are stored in
`validation/`; reports include nine repetitions, medians, peak process RSS,
response bytes and engine version. Those figures are measurements of the
current architectures, not an isolated test of Rust versus Swift as languages.

## Behavioral oracle and compatibility scope

The baseline is `SpecCodec` + `SpecValidator` + `ChartResolver` + `QueryRuntime`
+ `EChartsMapper`. Positive comparisons check ordered column names/types, row
count/order, typed cell values, NULL, option structure, series order and script
paths/source. Only floating values permit relative error `1e-9 * max(1, |x|)`;
all other values and structure compare exactly. Errors compare rejection,
not localized wording. The iOS host compares against exported Swift results,
not Rust results exported back to Rust.

Compatibility is limited to the tested CSV/ECharts graph. The prototype
validates every document supplied in a request and rejects non-CSV source
variants; do not pass a whole mixed-source workspace and assume unrelated
HTTP/database/HealthKit documents will be ignored. Dashboard and native/value
presentation remain on Swift. The probes establish a corpus, not blanket
YAML or SQL dialect equivalence.

Coverage includes the four ECharts recipes and changed parameters, multi-CSV
join/aggregation/window SQL, source/dataset references with explicit arguments
and target defaults, leading zeroes/commas/multiline CSV, empty queries, NULL,
bools, date/timestamp/time, decimal, UBIGINT/HUGEINT, lists/structs/maps, all
binding modes, color/NULL rules and inline/named scripts. Authored scripts are
recorded only while traversing the original configuration. Inserted CSV/query
and parameter values are returned directly and never traversed again. A `js:`
cell stays ordinary data.

Negative cases cover unknown/mistyped/missing/range/options parameters,
references/cycles, malformed SQL, quoted/commented/repeated parameters,
multi-statements, file/network readers and escape paths. Native tests also
check the 100 MiB input limit, 100,000 source/result row limits, and disabled
external access / extension autoload / autoinstall after trusted import.
DuckDB uses a 128 MB limit, two threads and no spill directory. This is a
DuckDB budget, **not a cap on total process RSS or JSON/renderer allocations**.

Rust now carries resolved YAML scalars through JSON before decoding structs,
matching Swift's protection against numeric-to-string coercion. Integral
floating numbers normalize as Swift's Int-first `Value` decoder does, without
converting quoted strings. Renderer-specific keys are rejected even when set
to YAML NULL. Comments and exact YAML text remain in Swift's existing
`WorkspaceStore` revision/save path; Rust does not re-encode or overwrite them.

The YAML probes report both parser acceptance and resulting payload equality.
Yams and serde_yaml_ng are not identical YAML dialects; plain implicit scalars must be handled explicitly before production. Merge
keys are now expanded by serde_yaml_ng; both tested decoders reject duplicate
keys. Anchors and explicit !!str tags match in the tested corpus.
A declared optional parameter with no value/default can pass definition
validation, but an ECharts binding to it fails in the current Swift runtime;
Rust deliberately follows that runtime behavior.

Two additional engine/extension probes expose remaining incompatibilities:
Swift's v1.6 development parser rejects nested block comments at execution
while Rust 1.4.4 accepts them; both scanners preserve the same nested comment
text and binding behavior. TIMESTAMPTZ formatting follows the local Asia/Shanghai
timezone in Swift's ICU-enabled engine (`2026-01-01 20:00:00+08`) but UTC in the
Rust bundle (`2026-01-01 12:00:00+00`). The core timestamps represent the same
instant, but the required text comparison fails. These are recorded as known
parity gaps, not normalized away or counted as successful exact comparisons.
The common corpus compares DATE, TIME and timezone-free TIMESTAMP exactly;
production must specify timezone/ICU configuration and resolve these gaps.

Rust additionally rejects a `data/` directory symlink pointing outside the
workspace. The current Swift root resolution can accept that case; this is an
intentional boundary hardening, recorded separately from matched rejections.
Concurrent path replacement remains unaddressed by canonicalization.

## Measurement boundaries

Cold means a fresh runtime/connection/import on each iteration, with no
application cache. It does not mean dropping OS filesystem caches. CSV fixture
generation and dylib loading occur before the timed loop. Peak RSS includes
fixture generation, libraries, allocator retention and the test host; it is a
process high-water mark over all nine iterations, not per-phase memory.

Swift acquisition reads CSV to a row-based DataFrame; its transform opens a
second DuckDB and appends those rows. Rust keeps imported tables in the same
DuckDB until query completion. Timings identify this architectural difference:
Swift `query_and_frame` includes re-import; Rust `open_and_import` includes CTAS.
Query execution and final frame conversion are grouped, not independently
profiled. Renderer preparation and response serialization are separate.

Rust reports request JSON decoding and response JSON encoding inside the ABI.
The Swift host measures request encoding, ABI call plus returned-buffer copy,
and Foundation response decoding. `ffi_copy_residual` is the difference
between that ABI interval and instrumented Rust phases: it also includes
version query, engine teardown, buffer allocation/free and envelope assembly.
It is **not** a precise memcpy-only measurement. Stage medians do not sum to
the median total. Swift's response encoding is a benchmark diagnostic, since
the shipping native runtime already owns its frame in Swift; total serialized
Swift/Rust figures should not be mistaken for shipping UI latency.

The Swift test executable statically contains Swift DuckDB. Rust test processes
additionally load Rust's dylib. Swift-only benchmark processes never dlopen
Rust. A separate Python/ctypes process loads only Rust DuckDB as a memory and
phase control, with different host/decode overhead. Engine versions returned
at runtime confirm the two engines remain distinct in the Swift/Rust tests.
These controls characterize the dual-engine effect; they do not prove an
exact production RSS saving.

Swift result-cache hits are separately measured after one unreported priming
query. Rust currently has no application cache; repeated stateless calls are
reported as cold, never as cache hits. A session cache is a required next step
before claiming cached-path parity.

## Measured release results

All figures below use the **common v1.6.0-dev11145 native engine**, nine
iterations per fresh case process, 100,000 input rows, 128 MB / two threads.
Aggregate produces 100 rows; large produces 100,000. `-payload` omits duplicate
frame transport. Totals include diagnostic JSON encoding; Rust also includes
C ABI transfer and host decoding. RSS is a process high-water mark.

| Case | Path | Median total ms | Peak RSS MiB | Response bytes |
| --- | --- | ---: | ---: | ---: |
| aggregate | swift | 142.98 | 114.8 | 5,554 |
| aggregate | rust | 51.60 | 94.8 | 5,802 |
| aggregate | swift-payload | 142.01 | 120.5 | 3,566 |
| aggregate | rust-payload | 51.73 | 96.9 | 3,826 |
| large | swift | 555.47 | 248.5 | 7,035,986 |
| large | rust | 140.23 | 363.8 | 7,036,222 |
| large | swift-payload | 421.13 | 232.9 | 4,667,966 |
| large | rust-payload | 120.36 | 263.1 | 4,668,223 |

Median phases in ms, payload-only paths:

| Case | Path | YAML/resolve | Open/import | Query/frame | ECharts | JSON encode | Host decode |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| aggregate | swift-payload | 0.459 | 105.37 | 32.81 | 0.03 | 0.17 | 0.00 |
| aggregate | rust-payload | 0.056 | 47.51 | 2.96 | 0.02 | 0.01 | 0.06 |
| large | swift-payload | 0.563 | 107.13 | 108.46 | 14.30 | 176.73 | 0.00 |
| large | rust-payload | 0.061 | 47.71 | 17.93 | 12.26 | 9.07 | 23.86 |

For aggregate payloads, the current Rust architecture takes 51.73 ms versus
142.01 ms for Swift; for large payloads, 120.36 versus 421.13 ms. These are
workload-specific improvements with native engine versions matched. The
major change avoids Swift CSV row materialization followed by a second
DuckDB import, and moves preparation/encoding into Rust. It does not show
that the language alone speeds up an equivalent algorithm.

Memory is **not consistently lower**. In the dual-engine Swift host, Rust
large full output peaks at 363.8 MiB versus Swift's 248.5; payload-only peaks
at 263.1 versus 232.9. The single-engine Python control peaks at 278.9 MiB
(full) / 197.2 (payload). This indicates host/duplicate-engine/JSON effects,
but differences in host object representation prevent attributing the whole
RSS delta to the duplicate native library. No production RSS saving is proven.

Payload-only reduces large Rust JSON from 7,036,222 to 4,668,223 bytes
(~34%) and dual-host RSS by ~101 MiB. In that path, Rust response encoding is
9.07 ms, Swift host decoding 23.86 ms, and ABI residual 9.22 ms (raw samples contain
exact medians). Serialization/host representation is already material, despite
only a ~1.9 MB input CSV. Prefer sending the final renderer JSON directly to
the WebView where possible, with a small validated control envelope, rather
than rebuilding a second full native object tree and re-encoding it. This
optimization has **not** been implemented/measured in the shipping host.

Swift primed result-cache medians: aggregate 0.53 ms, large 332.79 ms with
full diagnostic transport. Cache-hit results avoid acquisition/query but
still prepare/encode a large option. Rust has no equivalent cache-hit sample.
A native session cache remains essential for fair steady-state comparison.

The original bundled-engine results (v1.4.4 Rust vs v1.6 development Swift)
are retained in `validation/pipeline-benchmark`: aggregate full 34.92 vs
143.26 ms; large full 120.47 vs 558.72 ms. Rust's matched-engine aggregate is
~51.6 ms, demonstrating that version/build/extension choices materially affect
measurements. Do not explain the entire initial difference as a Rust benefit.

Behavior: nine opt-in tests passed for both default and common-engine runs,
including four recipes, changed parameters and complete 100,000-row output.
Sixteen successful Swift oracle fixtures were exported for iOS. The two
SQL/TIMESTAMPTZ probes **match with the common engine**, while the five
implicit YAML string-scalar probes still differ. A separate serial release
run passed all 107 regular Swift tests. The initial default-parallel run had
a MySQL migration assertion failure and did not finish after two minutes;
the migration test passed isolated and the full suite passed with
`--no-parallel`. No production Swift source was changed. See raw status/probes.

## Platform validation

| Target | Compile / link | Actual execution |
| --- | --- | --- |
| arm64 macOS 27.0.1 | Release dylib and archive built; minimum 14.0, SDK 27.0 | JSON C ABI, four recipes, boundary corpus, full/large payloads and benchmarks passed. |
| arm64 iOS 27 simulator | Release Rust archive statically linked into optimized Swift host; Rust minimum 17.0, host minimum 26.6, SDK 27.0 | All 16 exported Swift oracle fixtures matched schema, values and final payload; bundled DuckDB v1.4.4. |
| arm64 iOS device | Release Rust archive compiled and statically linked into optimized Swift host; Rust minimum 17.0, host minimum 26.6, SDK 27.0 | Unsigned compile/link artifact only; no physical-device execution yet. |

The simulator run uses an isolated UIKit test app, with the production app's
deployment target, not a modified Facetra application. It tests the complete
Swift invocation / YAML / reference / CSV / Rust-owned DuckDB / ECharts payload
route, including 100,000 output rows. It does not test ECharts drawing or native
permissions/lifecycle integration in the shipping app. Existing Swift/WebView
tests remain on their current runtime. Simulator and device latency/RSS have
not been benchmarked; macOS numbers cannot be generalized to those targets.

The initial simulator host failed to launch because SDK 27 requires UIScene
lifecycle support. The helper now uses a scene delegate/configuration and the
rerun passed. This was a host-launch failure, before usable pipeline results.
See [Apple's scene lifecycle migration guidance](https://developer.apple.com/documentation/uikit/transitioning-to-the-uikit-scene-based-life-cycle).

Physical-device runtime, signed distribution, sandbox/bookmark behavior,
background suspension/cancellation, jetsam, the supported older OS floors,
x86_64 macOS/simulator, Windows and Android remain unverified. Successful
cross-compilation alone does not establish any of those runtime properties.

`xcodebuild -create-xcframework` successfully packaged separate `ios-arm64`
and `ios-arm64-simulator` static-library slices with the C header/module map.
`vtool` verifies the Rust dynamic counterparts' platform/minimum/SDK and the
linked Swift hosts' 26.6 minimum. The archives use those same compiled objects.
The optional macOS common-engine control is kept out of this package.
Release Rust tests (one scanner + three pipeline tests), all-target Clippy
with warnings denied, rustfmt and helper syntax checks pass.

## Storage and linked artifacts

Observed after sequential host/simulator/device release builds and final checks:

| Build directory | Allocated size GiB (`du -sk`) |
| --- | ---: |
| Swift `.build` | 3.727 |
| Rust `target` (host + both iOS targets) | 2.489 |
| Common-engine Rust-only control cache | 0.539 |
| Combined build directories | 6.755 |

Libraries below already reside within those caches, except the relinked native
control library; do not add them again when calculating cache usage.

| Artifact | File size MiB |
| --- | ---: |
| macOS Rust static archive / dylib | 113.96 / 37.93 |
| iOS device / simulator static archives | 111.64 / 113.53 |
| Common-engine native dylib / Rust wrapper dylib | 75.42 / 2.18 |
| Linked device / simulator smoke executables | 43.93 / 44.19 |

Packaged directories, measured separately: XCFramework 225.19 MiB; unsigned
device smoke `.app` 43.93 MiB; ad-hoc simulator `.app` 53.02 MiB, of which
8.81 MiB is test fixtures. These are uncompressed development artifacts.
XCFramework packaging duplicates the archives into `build/`. No shipping
Facetra app/IPA size or App Store thinning/compression benefit was measured.
Archive bytes are not application download bytes. Exact byte counts and target
flags are retained in `validation/platform-and-size.json`.

Free space was checked before builds (initially ~12 GiB, later ~127 GiB after
external filesystem changes). This workflow did not remove other project or
global caches. Low-symbol release builds reuse one profile; the README explains
symbol/debugging and rebuild tradeoffs, shared CARGO_TARGET_DIR invalidation,
and project-scoped cleanup. The previous paused ~8.22 GiB figure was a different
cache state and is not directly comparable to these app artifacts.

## ECharts responsibility after migration

| Owner | Responsibilities |
| --- | --- |
| Rust | Raw YAML decode/validate/defaults, reference/with resolution, source import, bound DuckDB SQL, final type/NULL conversion; option assembly, field/fields/x-y/rows/params bindings, colors, series order, script paths/source. |
| Swift | Raw document storage/editor, SwiftUI/state, file/security scope, Keychain and native permissions, background invocation/task lifecycle/errors, WebView creation/destruction/theme/host messages. |
| JavaScript | Restore only explicitly listed script expressions to functions, ECharts drawing/resize/responsive layout/interaction. Never infer scripts from data strings. |

The stable renderer contract remains `{option, scripts:[{path, source}]}` and is
compatible with the existing WebView host. Normal production ECharts requests
should return that payload without a duplicate frame. The prototype's
`include_frame:false` supports measuring this path; it still constructs the
bounded final frame in Rust to prepare options. It is not streaming output.
Native/default/value chart presentation has not been ported by this experiment.

## Proposed production sequence

Use the results to approve an incremental shared-core migration, with explicit
gates, rather than switching the application based on successful compilation.

1. Pin a common DuckDB native revision/extension set, specify timezone behavior
   and resolve the documented SQL/YAML dialect
   differences. Add an enforced compatibility corpus for existing workspaces;
   preserve authored raw YAML/comments independently from the parsed AST.
2. Introduce a Rust workspace session with bounded source/table/result caches,
   structured errors, task IDs, cancellation/interrupt and resource ownership.
   Close path TOCTOU with host file capabilities. Repeat cold/cache/memory tests.
3. Adopt payload-only JSON for small charts; use measured size thresholds and
   a batch/buffer interface for large table results. Avoid shipping duplicate
   Swift/Rust DuckDB engines. Keep data import in Rust, never bounce source
   rows through Swift for SQL. Follow `SOURCE_INTERFACE.md` for new providers.
4. Validate device runtime (permissions, lifecycle, cancellation, jetsam),
   supported OS floors, x86_64 macOS if needed, and signed release packaging.
   Then integrate one opt-in CSV chart route behind a feature flag, with Swift
   fallback and the same acceptance corpus, before removing its Swift logic.
5. Expand CSV coverage, then native batch sources (HealthKit), HTTP/database/
   object-store adapters according to demand. Keep SwiftUI/WebView; no Tauri
   migration or UI rewrite is part of this proposal. Cross-compile and run
   Windows/Android conformance tests before promising those clients.

The main expected maintenance benefit is one spec/resolver/query/renderer data
implementation across hosts. Performance gains must remain workload-specific;
JSON boundaries and large prepared options can dominate after import becomes
cheaper. Missing cache/cancellation and unvalidated device behavior remain
production blockers even if the cold CSV pipeline compares successfully.
