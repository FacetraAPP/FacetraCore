# HTML, Vega-Lite and Markdown Notes

`renderer: html` and `renderer: vega_lite` share the existing source → Dataset →
chart pipeline on macOS and iOS. Details, dashboard panels, YAML previews and
macOS automation PNG exports use the same bundled offline WebKit hosts.
Dataset remains required; with one input, `dataset.sql` can be omitted.

## HTML

HTML accepts `option.html`, optional `option.css` and optional object/array
`option.data`. Data can contain ordinary constants, `{i18n: message}`,
`{from: params, name: parameter}`, and the existing typed Dataset bindings.

```yaml
chart:
  dataset: sales
  renderer: html
  option:
    data:
      rows: {from: sales, rows: true}
    html: |
      <ul>{{#rows}}<li>{{month}}: {{revenue}}</li>{{/rows}}</ul>
    css: |
      body { padding: 20px; color: var(--text); }
      li { padding: 8px; border-bottom: 1px solid var(--border); }
```

Mustache 4.2.0 resolves escaped `{{name}}`, dotted names, sections and list
iteration. Keep calculations, formatting and aggregation in Dataset SQL. The
host provides `--text`, `--muted`, `--card`, `--border`, and `--accent` theme
variables. Empty Dataset results can still display static markup or inverted
sections (`{{^rows}}No results{{/rows}}`).

Authored JavaScript is unsupported: `option.js` and `chart.scripts` fail
validation. HTML renders in an iframe sandbox without `allow-scripts` or
`allow-same-origin`; its CSP also prohibits scripts, network, forms and external
resources. Inline scripts and event handlers do not execute. The renderer
library and host are bundled with the application. These are technical limits,
not a guarantee of any App Store review decision.

## Vega-Lite

Vega 6.4.0 and Vega-Lite 6.5.0 are bundled locally. `chart.option` retains the
native Vega-Lite spec, including mark, encoding, transform, layer, facet,
concatenation, repeat and declarative interaction parameters. SVG output adapts
to the container and system theme; authored options take precedence.

```yaml
chart:
  dataset: sales
  renderer: vega_lite
  option:
    data:
      values: {from: sales, rows: true}
    mark: {type: bar, tooltip: true}
    encoding:
      x: {field: month, type: temporal}
      y: {field: revenue, type: quantitative}
```

Facetra resolves Dataset/parameter/i18n bindings before the spec reaches
Vega-Lite. Data types, nulls and SQL row order are retained; CSV numeric fields
should be explicitly cast in SQL. A native lookup transform's `from` object
remains native configuration. Dataset bindings must reference `chart.dataset`.
No JavaScript callbacks, `js:` compilation or `chart.scripts` are provided for
this renderer. Vega-Lite's own declarative expressions, such as filter and
calculate, retain their normal semantics. External data, images and URLs are
blocked; acquire data through Facetra sources instead.

Single and layered views default to Vega-Lite's responsive `width: container`,
so axes and legends share the full available card width without a fixed margin.
An authored width is preserved; faceted and concatenated views keep their own
geometry. See [Vega-Lite sizing](https://vega.github.io/vega-lite/docs/size.html).

The ECharts host uses a four-point default horizontal margin and ECharts 6's
automatic outer bounds to fit axis labels and names, without also forcing
`containLabel` inside an authored axis margin. Endpoint labels align inward.
Explicit grid/label settings and media rules retain precedence; newly created
recipes inherit these host defaults. See [ECharts grid bounds](https://echarts.apache.org/en/option.html#grid.outerBoundsMode).

Vela initially fits the latest 200 bars with half a bar of space at each edge,
keeping complete candles visible without a two-bar forecast area. Its price-axis
gutter remains available for labels. Resizing keeps the initial history fitted;
after the user pans or zooms, their selected viewport is preserved.

## Notes and Dataset inspection

Chart and dashboard Notes now use the shared Markdown reader: headings, inline
emphasis, links, lists, quotes and fenced code. A fenced `mermaid` block uses
bundled Mermaid 11.12.0 in strict security mode. Themes follow the application;
invalid syntax shows an error and the source remains available for inspection.
Notes continue to be authored/persisted as the same string or localized message.

The chart's existing table action opens the shared data inspector. Its selector
lists **Dataset Result** above a separated **Data Sources** group. Dataset Result
always shows a table of the actual transformed output, independent of source
`display: df | json | text`. Selecting a source preserves its original preview
mode. The tab retains its selection when toggling chart/table. Refresh retains
visible data until success, reports errors and supports retry; changing the
selection or query clears the previous result. Source previews retain their
existing frozen-page loading and cell inspection/copy behavior.

The Discover Example workspace includes 12 Vega-Lite examples, two HTML views
and a dashboard. [Discover PR #17](https://github.com/FacetraAPP/Discover/pull/17)
moves this gallery into the existing Example workspace after the renderer
support in Facetra PR #78.

## Validation evidence

See the screenshot and interaction record in `Docs/ui-ux-guidelines.md`.
The generated Rust contract checks, Rust tests/lints, native builds, real
WebKit integration tests, isolated iOS flows and Discover catalog validation
were run. Four existing checks against the pinned Discover submodule still fail
because of legacy workspace localization fields and byte comparisons with older
Gallery/Vela fixtures; the submodule pointer is unchanged by this feature.
