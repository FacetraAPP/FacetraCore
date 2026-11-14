# Shared Rust application core

Rust owns every source and dataset execution path: CSV, HTTP, PostgreSQL,
MySQL, SQLite, request templates, raw previews, reference resolution, shared
acquisition/cache, DuckDB SQL and typed results. Swift owns platform file
permissions, Keychain, UI, payload decoding and WebKit / value-card presentation.
Rust prepares complete ECharts/HTML/Vega-Lite/Vela/value payloads for both the app and standalone pipeline, including resource-local
`i18n` references; its optional `locale` request field selects the language (default
`en`), with each message falling back to its catalog’s `defaultLocale`. Translated
text remains data even when it starts with `js:`. HealthKit has been removed.

See [SOURCE_INTERFACE.md](SOURCE_INTERFACE.md) for the current ownership/data
contract and [MIGRATION.md](MIGRATION.md) for verification results. DuckDB is
pinned to bundled 1.4.4, Rust to 1.90.0.

## Build and checks

```sh
Tools/check-quality.sh
cargo +1.90.0 test --release --locked -p facetra-pipeline -j 4
```

Core checks run formatting, Clippy with warnings denied, contract drift checks,
all Rust tests and the optional browser screenshot smoke test. HTTP and database
wire fixtures run only on loopback, using generated TLS certificates and no user
credentials. The optional real-server suite starts isolated temporary PostgreSQL
and MySQL instances; it never uses an existing workspace database.

The Facetra app repository can consume the rolling `main-latest` prerelease from
this repository instead of compiling the Rust source locally. The release
contains `FacetraPipeline.xcframework.zip`, the renderer and contract support
bundle, a manifest and SHA-256 checksums. The repository remains available as
the `Rust` submodule for source-level development and debugging.

To build the same artifacts locally on macOS with Xcode installed:

```sh
Tools/build-apple-artifacts.sh
```

The script produces macOS arm64/x86_64, iOS device arm64 and iOS simulator
arm64/x86_64 slices, then packages them under `build/apple/`. Native DuckDB
compilation is expensive; the script builds platform slices sequentially. The
shared core deployment floors remain macOS 14/iOS 17.

For iOS builds, the script gives host build scripts and procedural macros the
macOS SDK; Rust and the `cc` crate select the iOS SDK from the target triple.
Clearing `SDKROOT` under Xcode can fail with `ld: library 'System' not found`
when those host tools are not already cached. To verify a cold simulator build
without clearing existing caches:

```sh
RUST_BUILD_CHECK=$(mktemp -d "${TMPDIR:-/tmp}/facetra-rust-build.XXXXXX")
cargo +1.90.0 build --release --locked --target aarch64-apple-ios-sim \
  --target-dir "$RUST_BUILD_CHECK" -p facetra-pipeline
```

## Session lifecycle

The owned session and cancellation handles are declared in
`include/facetra_pipeline.h`. Normal source/dataset requests use ABI v1
`execute`/`source`. Acquisition runs outside the DuckDB mutex; queries serialize
one retained engine and latest result. Opaque host revisions invalidate source
and result caches. Cancellation is per call and interrupts active DuckDB work;
source flight lifetimes are independent of any one caller. Destruction waits
for in-flight workers; free every response exactly once with
`facetra_pipeline_free`.

The app requests final renderer JSON with `execute.render` or `render_frame`;
Swift does not map typed DataFrames into chart series. `source_page` returns bounded pages from explicitly
released frozen snapshots. Swift template inspection and Rust execution share
the `facetra-contract` scanner. The low-level `query` API remains for compatibility
and historical fixture comparisons. `facetra_git_run` exposes the independent libgit2
workspace implementation used by the app.

## Apple validation

App-level macOS and iOS builds run in the
[Facetra repository](https://github.com/FacetraAPP/Facetra), where this
repository can be checked out as `Rust/`. The `Apple artifacts` workflow builds
and publishes the rolling prerelease after `Quality` succeeds on `main`. To
build individual iOS slices manually:

```sh
cargo +1.90.0 build --release --locked --target aarch64-apple-ios -p facetra-pipeline
cargo +1.90.0 build --release --locked --target aarch64-apple-ios-sim -p facetra-pipeline
```

`VALIDATION.md` and `validation/` are historical comparison evidence, not the
current runtime design.
