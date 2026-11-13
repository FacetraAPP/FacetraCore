# Chart and Dashboard localization

Charting Contract v1 supports an optional `i18n` catalog on each Chart or
Dashboard. Display fields use either a fixed string or an explicit reference:

```yaml
i18n:
  defaultLocale: en
  messages:
    title: {en: Monthly revenue, zh-Hans: 月度收入}
metadata:
  id: monthly_sales
  title: {i18n: title}
```

## Rules

- `metadata.title`, `metadata.notes`, parameter labels and Dashboard tab titles
  accept a string or an object containing exactly one `i18n` key.
- ECharts option display fields use the same references. References cannot
  replace field bindings, renderer types or other structural values.
- Every key must contain a nonblank translation for `defaultLocale`. Missing
  keys report `i18n.missing`; incomplete defaults and ambiguous locales report
  `i18n.invalid`.
- Locale matching is case-insensitive and falls back by removing trailing
  subtags. For example, `zh-Hant-TW` can fall back through `zh-Hant` to the
  declared default. Simplified and traditional scripts do not fall back to each
  other.
- Authored values, SQL, HTTP templates, data fields and script strings are not
  translated. Localized text is always data, including text beginning with
  `js:`.
- Each resource owns its catalog. A chart embedded in a Dashboard does not
  inherit the Dashboard catalog.

`workspace.yaml` titles and descriptions are fixed identity text. Workspace
metadata rejects `i18n`, `localizations` and reference objects; see the
[Workspace metadata contract](../Contracts/workspace/v1/specification.md).

Valid fixtures:

- [Localized chart](../Contracts/charting/v1/fixtures/valid/localized.yaml)
- [Localized dashboard](../Contracts/charting/v1/fixtures/valid/localized-dashboard.yaml)
