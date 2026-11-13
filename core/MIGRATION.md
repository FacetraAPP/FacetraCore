# 当前运行时说明

本文保留历史迁移与测量记录。后续稳定身份、统一 ECharts 编译及分页预览的实现与新测量见 [架构整理与维护边界](../Docs/architecture-maintenance.md)；当前接口以 [SOURCE_INTERFACE.md](SOURCE_INTERFACE.md) 为准。

# Source and dataset migration validation

Validated on October 12, 2026 with Rust 1.90.0, Xcode 27.0, macOS 27.0.1,
iOS 27.0 simulators and bundled DuckDB 1.4.4. This records PR #32's source
migration. [VALIDATION.md](VALIDATION.md) and `validation/migration/` retain
historical prototype / earlier application measurements.

## Ownership and compatibility

Rust now performs CSV, HTTPS, PostgreSQL, MySQL and SQLite acquisition,
request templates, raw previews, source caching, cancellation, reference
resolution and dataset SQL. Swift supplies scoped file access and Keychain
values, decodes the final typed DataFrame and renders native Charts / ECharts.
The production path is `execute` → DataFrame → Swift mapper. The old low-level
`query` / Rust ECharts API remains only for compatibility and fixture parity.

HealthKit has been removed from the application, permissions, capabilities,
settings, schema and bundled examples. Existing `source: healthkit` YAML
reports that the provider is no longer supported; replace it with CSV, HTTP
or a database source. The example catalog now has eight charts and three
dashboards. The Discover gitlink points to the companion removal in
[Discover PR #1](https://github.com/FacetraAPP/Discover/pull/1); the public
catalog changes when that PR is merged and published.

## Data verification

- Rust: 51 unit tests, eight ABI execution tests and three pipeline tests;
  the separately enabled real-database integration test also passes.
- Swift: 185 tests pass; three opt-in tests are skipped in the core run.
  Covered behavior includes source/dataset references and parameters,
  source previews, native/ECharts rendering parity, source sharing,
  independent cancellation, credential invalidation, retries, SQL binding,
  imports and the bundled/published example comparison.
  The benchmark is run separately, real databases are tested by the isolated
  Rust suite, and the live public-catalog test awaits companion publication.
- Five YAML editor tests and nine Discover catalog tests pass.
- Ten focused iOS UI tests pass: chart details, source previews, wide tables,
  HTTP charts, example isolation and Discover imports. Thirteen additional
  UI cases passed in the preliminary run; the full UI suite was not completed.
- macOS universal and unsigned iOS device Release builds pass.
- The standalone iOS simulator host passes 48 exported data fixtures and
  session/execute ABI checks. No physical-device execution is claimed.

Real PostgreSQL 17 and MySQL 8.4 instances are initialized in temporary
folders, bound to loopback and removed after the test; no existing databases
or user credentials are used. Tests verify signed 64-bit values, exact
DECIMAL text, UTF-8, leading-zero strings, NULL, parameter injection resistance,
read-only transactions, deadlines and source → DuckDB aggregation. PostgreSQL
UUID, money and UTC timestamps are checked against the real server as well as
wire fixtures. SQLite tests cover read-only access, all-NULL/empty declared
types, WAL-consistent backups, file boundaries and cancellation.

Generated-certificate HTTPS fixtures exercise GET/POST, query/form/typed JSON,
credential templates, redirects, untrusted TLS, byte limits, raw recordsPath
previews, shared work and retry/cancellation. Database wire fixtures exercise
both drivers, TLS refusal, binary types and timeout behavior. Boundary tests
cover 100,000 rows, source/result byte limits and invalid ABI requests.

## Coverage

`Tools/check-rust-coverage.sh` measures production Rust only, excluding test
modules and dependencies. CI runs the real database suite and uploads JSON
and HTML reports. The checked-in summary is in
[validation/sources/coverage-summary.json](validation/sources/coverage-summary.json).

| Production scope | Lines | Regions |
| --- | ---: | ---: |
| Sources | 97.05% | 92.37% |
| Data pipeline | 87.04% | 84.15% |
| Entire Rust core | 88.80% | 84.28% |

The gates require source modules individually to reach 90% lines; the source
group also requires 85% regions. The data pipeline and entire core must reach
85% lines / 80% regions. Stable Rust 1.90 reports LLVM lines and regions, not
branch percentages. These are not branch-coverage claims.

LLVM's HTML report emits four profile hash warnings for alternative ABI
function mappings (`free`, cancellation/session creation and session destruction).
All four functions have recorded integration-test hit counts in the JSON report;
source coverage is unaffected. Reported values are retained without suppressing
warnings or excluding these production functions.

## Measurements

Each scenario imports a 100,000-row CSV. Aggregate output contains 100 rows;
large output contains all 100,000 rows. Every sample acquires sources, runs or
reuses dataset SQL, transfers and decodes the typed frame, and prepares ECharts
in Swift. Cold calls use a fresh session; snapshot reuse changes a dataset
parameter; result hits reuse the Rust query result but still transport the
frame and prepare the Swift payload. Nine samples are recorded per scenario.

| Scenario | Cold median | Snapshot reuse median | Result hit median |
| --- | ---: | ---: | ---: |
| Aggregate | 37.413 ms | 4.061 ms | 6.345 ms |
| 100,000-row output | 920.463 ms | 876.736 ms | 885.482 ms |

Full samples, payload sizes and the simulator result are in
[validation/sources/](validation/sources/). These are local measurements,
not a cross-language speed comparison. The bounded JSON boundary includes
serialization costs; it is not Arrow or zero-copy transport.

## Reproduction

```sh
FACETRA_RUST_FIXTURES="$PWD/build/rust-fixtures" Tools/check-quality.sh core
# Requires cargo-llvm-cov 0.9.1, llvm-tools-preview, PostgreSQL and MySQL binaries.
FACETRA_COVERAGE_DATABASES=1 Tools/check-rust-coverage.sh
Tools/benchmark-pipeline.sh
FACETRA_SIMULATOR=<ios-27-simulator-uuid> Tools/validate-rust-ios.sh simulator
```

Initialize the pinned Discover submodule for the published-example parity test,
or set `FACETRA_DISCOVER_EXAMPLE` to that revision's extracted `example` folder.
The opt-in live public-catalog test should run after the companion Discover PR
is published. See [SOURCE_INTERFACE.md](SOURCE_INTERFACE.md) for the ABI and
[the data pipeline](../Docs/data-pipeline.md) for application behavior.
