# Workspace metadata text v1

Workspace `title` and optional `description` are fixed authored content. They do
not change with App language and do not use Chart/Dashboard message catalogs.

```yaml
title: 示例 · Examples
description: Example charts and dashboards / 示例图表与看板
license: MIT
```

The generated [metadata text schema](metadata.schema.json) requires a nonblank
string `title`. `description` may be absent, null, empty or a string. Neither
field accepts `{i18n: key}` or an inline locale map. Root `i18n` and `localizations`
are forbidden even when empty or null. This removes the former workspace summary
map without a compatibility reader or migration. Authors keep their intended
fixed title/description strings and remove the translation fields.

The schema covers this text boundary only; other portable and device metadata
remain validated by their existing owners. `Tools/check-contract.sh` checks the
generated asset for drift. Rust's `workspace_metadata` operation takes `document`,
returns an empty success object, and reports structural failures through
`structure.invalid` diagnostics with JSON Pointer paths. Forbidden fields point
to `/i18n` or `/localizations`; errors never expose authored values.

Swift applies the contract when decoding workspace manifests and registrations.
Portable writes contain fixed strings. Discover validates published manifests
against the same rules, copies their fixed strings into workspace catalog
headings, and emits no workspace `localizations` map. App language changes retain
workspace names, paths, titles and descriptions. Import preserves these values separately from device-local names.

Workspace manifests no longer declare `requiredConnections`. Charts infer aliases
from the sources used by their dataset and parameter options, following cross-chart
source and dataset references. Older declarations are ignored when reading and
omitted on new writes; opening or renaming a workspace preserves its file bytes.

[Charting Contract v1](../../charting/v1/specification.md#localized-display-text)
continues to support per-resource `i18n` on Charts and Dashboards. Their generated
catalog `localizations` snapshots remain supported; this workspace rule does not
remove resource translations.
