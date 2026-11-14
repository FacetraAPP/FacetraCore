# FacetraCore

Rust core, YAML contract, and standalone rendering CLI for Facetra.

This repository is the home of:

- `contract/` — the versioned Chart/Dashboard YAML contract, models, schemas,
  validation and reference resolution.
- `core/` — the execution engine: CSV, SQLite, HTTP and database sources,
  DuckDB SQL, rendering payload compilation, and the C ABI used by the apps.
- `cli/` — `facetra-render`, a standalone renderer for CI, agents and local
  automation.
- `renderers/` — the offline ECharts, Vela, Vega-Lite, HTML and value hosts used
  by the renderer CLI and the Facetra app.

`FacetraCore` is not the Mac App CLI. The `facetra` command that performs local
chart/dashboard CRUD through Facetra automation remains in the
[Facetra](https://github.com/FacetraAPP/Facetra) repository.

## Build

Rust 1.90.0 is pinned.

```sh
cargo check --locked
cargo test --locked
```

The renderer CLI needs Node.js and Playwright:

```sh
npm ci --prefix renderers
./renderers/node_modules/.bin/playwright install chromium
```

## Apple artifacts

Every successful `Quality` run on `main` builds a rolling prerelease at
[FacetraCore main](https://github.com/FacetraAPP/FacetraCore/releases/tag/main-latest).
It is marked as a prerelease and is overwritten by later main builds.

The release contains:

- `FacetraPipeline.xcframework.zip` with macOS arm64/x86_64, iOS device arm64 and
  iOS simulator arm64/x86_64 slices plus the C header and module map.
- `FacetraCoreSupport.zip` with renderer assets, Chart/Dashboard contracts,
  examples, headers and third-party notices.
- `FacetraCore-main.json` with the source commit, workflow run, deployment
  floors and SHA-256 checksums.
- `SHA256SUMS` for download verification.

Build and package the same artifacts locally on macOS with Xcode installed:

```sh
Tools/build-apple-artifacts.sh
```

Outputs are written to `build/apple/`. The rolling `main-latest` release is a
mutable development channel; consumers must verify `SHA256SUMS` or the manifest
checksum before linking the static libraries.

## Render a chart

```sh
cargo run -p facetra-render -- chart payload \
  --workspace /path/to/workspace --id example_price

cargo run -p facetra-render -- chart render \
  --workspace /path/to/workspace --id example_price \
  --out /tmp/example_price.png --width 1200 --height 700
```

The CLI also accepts a draft YAML file:

```sh
facetra-render chart render --workspace . --file draft.yaml \
  --params '{"symbol":"600519.SH"}' --out chart.png
```

Workspace operations in the renderer CLI are read-only. They discover charts,
load YAML, resolve cross-file references, validate the contract and render
payloads. They do not write, move, delete or register workspaces, read Keychain
values, or perform Git/S3 operations.

## Render every chart in a workspace

```sh
facetra-render --json batch --workspace . --out-dir previews \
  --width 1200 --height 700 --renderer-dir renderers
```

The command writes PNGs plus a JSON manifest containing each resource ID,
renderer, relative path and SHA-256.

## Discover previews

`.github/actions/render-previews` is a composite action for Discover and other
content repositories. It installs the pinned browser dependency, builds
`facetra-render`, and writes a preview manifest. See
[Docs/render-cli.md](Docs/render-cli.md) for the complete contract.

## License

Original code is available under either the MIT or Apache-2.0 license. Vendored
renderer assets retain their upstream licenses and notices; see
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md), [LICENSE-MIT](LICENSE-MIT)
and [LICENSE-APACHE](LICENSE-APACHE).

