# facetra-render

`facetra-render` is the standalone CLI in this repository. It executes YAML
through `facetra-pipeline` and captures the final renderer payload with a
headless Chromium browser. It never launches or connects to the Facetra Mac app.

The Mac app has a separate CLI named `facetra` for local chart/dashboard CRUD
through the app's automation service. That command remains in the
[Facetra repository](https://github.com/FacetraAPP/Facetra); the two CLIs do not
share a command surface or release process.

## Workspace model

The CLI accepts a directory containing:

```text
workspace.yaml
charts/<optional folders>/<id>.yaml
dashboards/<optional folders>/<id>.yaml
data/
connections/
```

The renderer workspace layer is read-only:

- scan `charts/` and `dashboards/`
- index charts by `metadata.id`
- read YAML and compute SHA-256 revisions
- load the full chart document set for references
- validate the contract

It deliberately does not implement resource CRUD, workspace registration,
directory grants, Keychain, Git/S3 sync or atomic saves. Those remain in the
Mac app and the `facetra` app CLI.

## Commands

```text
facetra-render [--json] workspace inspect [PATH]
facetra-render [--json] workspace validate [PATH]
facetra-render [--json] chart payload (--id ID | --file FILE) [OPTIONS]
facetra-render [--json] chart render (--id ID | --file FILE) --out PNG [OPTIONS]
facetra-render [--json] batch --workspace PATH --out-dir DIR [OPTIONS]
```

Common options:

- `--workspace PATH`: workspace root or a descendant; defaults to `.`
- `--id ID`: select an existing chart
- `--file FILE`: use a draft chart YAML; its ID selects the chart
- `--params JSON`, `--params-file FILE`: scalar parameter values
- `--locale LOCALE`: resource-localized text; defaults to `en`
- `--width`, `--height`, `--scale`: logical size and pixel scale
- `--renderer-dir DIR`: directory containing `capture.mjs` and renderer hosts
- `--force`: replace an existing PNG

`chart payload` is useful to agents and diagnostics: it returns the final
client-neutral renderer JSON without launching a browser.

Batch output is deterministic for the same YAML, data, renderer assets and
browser version. The manifest has this shape:

```json
{
  "ok": true,
  "result": {
    "workspace": "/path/to/workspace",
    "width": 1200,
    "height": 700,
    "scale": 1,
    "locale": "en",
    "resources": [
      {
        "id": "example_price",
        "title": "股价与均线",
        "renderer": "echarts",
        "path": "example_price.png",
        "sha256": "..."
      }
    ],
    "failures": []
  }
}
```

The process exits with `0` when every requested chart rendered, `1` for a
runtime or rendering failure, and `2` for invalid arguments. With `--json`,
errors are one JSON object.

## Renderer hosts

| Renderer | Asset directory | Headless engine |
| --- | --- | --- |
| ECharts | `renderers/ECharts` | Chromium |
| Vela | `renderers/Vela` | Chromium |
| Vega-Lite | `renderers/VegaLite` | Chromium |
| HTML | `renderers/HTML` | Chromium |
| Value card | `renderers/Value` | Chromium |

`renderers/capture.mjs` starts one Chromium process per batch, injects the
offline scripts after the document body exists, waits for the renderer status,
and captures a PNG at the requested device scale.

## Local data and credentials

The first version is optimized for reproducible offline previews:

- CSV and SQLite under the workspace `data/` directory
- no Keychain access
- no App Group, signing or Mac app requirement

The PNG capture itself is offline once Playwright and Chromium are installed.
Source execution follows the YAML: an HTTP or database-backed chart can still
contact its configured endpoint. The CLI has no Keychain or App Group access, so
it cannot reuse credentials stored by the Mac app. Discover CI should render
only committed public data and should not receive production credentials.

## Discover and PR previews

The reusable action is:

```yaml
- uses: FacetraAPP/FacetraCore/.github/actions/render-previews@<pinned-sha>
  with:
    core-ref: <same-pinned-sha>
    workspace: workspaces/example
    out-dir: previews/example
```

The action writes `manifest.json` in `out-dir`. Discover should use that
manifest to update `DiscoverFile` entries (`path`, `size`, `sha256`) and should
pin the action to a commit or release tag.

For pull requests, render only changed YAML resources and upload the PNGs as
workflow artifacts. A follow-up step can post before/after images; the CLI
itself remains a deterministic local tool. Pin both the action and `core-ref` to
the same immutable commit or release tag.

## Dashboard scope

The first version renders individual charts. Dashboard previews require
resolving panel bindings and composing multiple chart payloads into a page;
that belongs in a follow-up `dashboard` command or a separate renderer module.

