# ECharts 图表与 YAML

Chart 支持 `renderer: echarts`。应用内打包 Apache ECharts 6.1.0，在 macOS / iOS 的 WebKit 中渲染，不依赖 CDN。独立图表、看板面板和 YAML 预览共用同一 renderer；取数、参数、DuckDB SQL、source/dataset 引用和缓存沿用现有[数据协议](data-pipeline.md)。

Discover 的 [图表原型库](../Discover/workspaces/example/README.md) 覆盖 23 种核心 series，包括 6.x 新增的 chord，以及 matrix、calendar、polar 等坐标示例。全部使用少量离线虚构数据，无需参数或连接。GL 是独立扩展，当前 SVG 渲染器未包含它。升级采用 6.x 默认主题；原有 YAML 的显式颜色、轴与布局继续生效。

创建图表时可选择「渲染引擎 → ECharts」，导入 CSV / HTTP 或引用 source 后生成可编辑的 YAML。线、面积、柱和饼是创建表单提供的起始配方；编辑 `chart.option` 可以配置散点、K 线、热力图、雷达、仪表盘等 ECharts 图表，不需要添加应用代码。

## 最小配方

```yaml
params:
  - {name: target, label: 目标, type: number, default: 100}
sources:
  sales: {source: duckdb, file: data/sales.csv}
dataset:
  id: sales
  sources: [{id: raw, ref: '#/sources/sales'}]
  sql: SELECT month, CAST(revenue AS DOUBLE) AS revenue FROM raw ORDER BY month
metadata:
  id: monthly_sales
  title: 月收入
chart:
  dataset: sales
  renderer: echarts
  option:
    tooltip: {trigger: axis}
    xAxis: {type: category, data: {from: sales, field: month}}
    yAxis: {type: value}
    series:
      - type: bar
        data: {from: sales, field: revenue}
        markLine:
          data: [{yAxis: {from: params, name: target}}]
```

`option` 必须是非空对象，其内部保留 ECharts 原生配置结构。旧的 `plot/x/y/average/color/x_end` 字段已移除；`value` 只属于数值卡片。ECharts option 自身的 `dataset` 是渲染器的数据组件，顶层 `dataset` 是 Facetra 的 SQL 管线，两者可以配合使用。

## 通用绑定

绑定可出现在 option 树的任何位置，`from` 使用本文件 `chart.dataset` 的 ID。绑定对象只能包含对应字段；字段名、Dataset 引用或脚本名错误会报错，不会静默返回空数据。

| 配置 | 替换结果 / 用途 |
| --- | --- |
| `{from: sales, field: revenue}` | 按 SQL 行顺序返回单列数组 |
| `{from: sales, x: month, y: revenue}` | 返回 `[x, y]` 数组，适合时间轴折线和散点 |
| `{from: sales, fields: [month, revenue, profit]}` | 按列顺序返回多列数组，适合 K 线、热力图和多维散点 |
| `{from: sales, rows: true}` | 返回完整行对象数组，适合 `option.dataset.source` 配合 `series.encode`，也可直接绑定包含 `name/value` 列的饼图数据 |
| `{from: params, name: target}` | 替换为参数当前值，保留 string/number/boolean 类型 |
| `{from: script, name: currency}` | 编译并使用 `chart.scripts.currency` 的 JS 表达式 |

数值、布尔值、文本、日期与 SQL NULL 均保留，行顺序由 SQL 决定。日期 / 时间戳传递为字符串；时间轴也可以在 SQL 中生成 `epoch_ms(...)`。大整数 / Decimal 沿用 DataFrame 的文本表示。颜色比较需要 SQL 输出数值列，CSV 原始文本应显式 cast。

列绑定还可增加 `color_by_sign: true`，按最后一列正负染为绿 / 红；或增加 `color_by: [close, open]`，按两列比较染色。两种方式互斥，NULL 不参与比较，不被视为零；整行绑定不使用这两个字段。自定义配色、条件和标记可使用 ECharts 原生 `visualMap`、`itemStyle`、`markLine` 或 formatter。

Dataset ID `params` 和 `script` 为绑定保留名。`field`、`fields`、`x/y`、`rows` 必须四选一。绑定结果不会再次被当作 option 配置解析，数据中的 `js:` 文本保持文本。

## 哪些能力直接靠配置

| 能力 | 配置位置 |
| --- | --- |
| 筛选、聚合、透视、均线、比例、排序、行数窗口 | `dataset.sql`，使用 `$parameter` 驱动参数 |
| 共享数据获取、共享转换结果 | `sources.*.ref` / `dataset.ref` 与显式 `with` |
| 多序列、堆叠、双轴、多 grid、面积填充 | ECharts 的 `series/stack/yAxis/grid/areaStyle` |
| 图例切换、悬停 / 轻触提示、缩放、坐标轴联动 | `legend/tooltip/dataZoom/axisPointer` |
| 业务颜色、色阶、目标线、标签、单位 | `color/itemStyle/visualMap/markLine/label/axisLabel` |
| 参数控制展示属性 | `{from: params, name: ...}`，如标签开关、目标值、坐标轴范围 |
| 格式化函数、custom series 绘制 | `chart.scripts` 或 option 中的 `js:` 表达式 |
| 明暗主题、尺寸适配、renderer 生命周期和错误显示 | 应用通用 renderer；YAML 显式 option 优先 |

新的业务图表应通过 SQL 和 option 实现。增加数据源或可供所有图使用的通用渲染能力时才需要应用代码。

默认不显示 `dataZoom` 滑块；`inside` 保留鼠标 / 触摸缩放，需要显示滑块时在对应配置中显式写 `show: true`。创建表单与测试配方均使用隐藏滑块。

## 手机与小尺寸适配

renderer 默认使用可滚动图例、避免重叠的坐标轴标签、限制在容器内的 tooltip。宽度 ≤ 480 时使用较小字号和较少的刻度，手机独立查看页给 ECharts 400pt 高度。图表与看板列表不加载数据或渲染预览，打开详情后才渲染图表。

列表中的图表类型根据静态 `series.type` 配置识别，共用短名称、图标和配色；中文标签最多 3 个汉字，英文省略 Chart 等重复词。`line` 配合 `areaStyle` 显示为「面积图 / Area」，柱线混合与 K 线组合分别显示为「柱线图 / Bar+Line」与「K线+ / Candles+」。`baseOption`、响应式和时间轴选项中的显式类型也参与识别；脚本或未知类型显示为「自定义 / Custom」，不为识别类型执行脚本或取数。

手机默认轻触显示 tooltip，避免滚动时提示框遮住图形。`inside` 缩放保留双指手势，单指拖动用于页面滚动；需要图内单指平移时可显式配置 `moveOnMouseMove: true` 和 `preventDefaultMouseMove: true`。

复合图的布局继续在 YAML 中配置，renderer 支持 ECharts 原生 `media`，不需要为每种图在 Swift 中增加分支。按**容器**宽高匹配，旋转或窗口缩放时重新应用，作者的 media 规则在通用默认值之后应用。例如：

```yaml
chart:
  # 其他字段省略
  option:
    # 通常的 series、xAxis、yAxis 等
    media:
      - query: {maxWidth: 480}
        option:
          xAxis: {axisLabel: {interval: 1}}
          yAxis: [{splitNumber: 3}, {splitNumber: 3}]
      - query: {maxHeight: 190}
        option:
          legend: {show: false}
          grid: {top: 8, bottom: 18}
```

四个测试配方提供具体规则：手机热力图通过 tooltip 查看数值，减少单元格文字；K 线在小高度容器中隐藏成交量副图；双轴图减少刻度和月份标签。更复杂的图建议为宽屏与窄屏分别给出布局规则，留足 `grid` 边距，避免把全部桌面信息挤入手机。

## JavaScript formatter

```yaml
chart:
  # 其他 chart 字段省略
  scripts:
    currency: "(v) => v == null ? '—' : '¥' + Number(v).toFixed(2)"
  option:
    tooltip:
      valueFormatter: {from: script, name: currency}
    yAxis:
      axisLabel: {formatter: "js: (v) => (v / 1000).toFixed(1) + 'k'"}
```

脚本必须是自包含的单个 JS 表达式，可以是箭头函数、`function (...) { ... }` 或表达式值。脚本名须为 JS 标识符。引用在保存 / 校验时检查，JS 语法及运行错误由预览显示，修复后可重试。脚本只在临时 WebKit 文档中执行；该文档禁止网络和外部资源，没有工作区文件或 Keychain 访问接口。事件回调 `chart.on(...)` 不属于 option，目前不提供配置接口。

## UI 测试工作区

UI 测试运行时创建「测试空间」，通过普通文件加载「ECharts · 配置示例」看板与 `Example/ECharts` 目录中的图表。四张图均可离线运行，支持编辑 YAML；测试结束后清理。用户可从发现页安装这些配方。

| 图表 | 配方 | 展示的通用能力 |
| --- | --- | --- |
| 股价与均线 | [example_echarts_price.yaml](../FacetraUITests/Fixtures/ExampleWorkspace/charts/Example/ECharts/example_echarts_price.yaml) | 与原生图共享 Dataset、时间轴、多序列、NULL 均线、命名 formatter、手势缩放 |
| 收入与利润率 | [example_echarts_business.yaml](../FacetraUITests/Fixtures/ExampleWorkspace/charts/Example/ECharts/example_echarts_business.yaml) | 堆叠柱 + 折线、双轴、参数目标线与标签开关 |
| K 线与成交量 | [example_echarts_candlestick.yaml](../FacetraUITests/Fixtures/ExampleWorkspace/charts/Example/ECharts/example_echarts_candlestick.yaml) | OHLC 列顺序、多 grid、联动缩放、均线、成交量涨跌配色 |
| 渠道月度热力图 | [example_echarts_heatmap.yaml](../FacetraUITests/Fixtures/ExampleWorkspace/charts/Example/ECharts/example_echarts_heatmap.yaml) | 跨文件 source 引用、宽表转长表、三列绑定、visualMap、SQL 参数切换指标 |

配方中的 `example_price.yaml` 对应 [现有股价示例](examples/example_price.yaml)。配方作为普通 YAML 在隔离 UI 测试工作区中创建，编译后的 App 不包含这些文件。12 行[经营数据](../FacetraUITests/Fixtures/ExampleWorkspace/data/example/echarts-sales.csv)和 40 行[OHLCV](../FacetraUITests/Fixtures/ExampleWorkspace/data/example/echarts-ohlcv.csv)均为虚构数据，使用 `python3 Tools/generate-example-csv.py` 可复现。

沿用 Spectra 的 `chart.option`、`from/field/fields/x/y/rows`、颜色绑定、`scripts` 与 `js:` 写法。Facetra 的 `renderer: echarts` 必须显式声明；参数继续使用有类型的列表和 `$name` SQL 绑定，不引入 Jinja。`kind`、Spectra 的其他 renderer 与 Dashboard 布局不能直接复制。

参考：[ECharts option](https://echarts.apache.org/en/option.html)、[dataset 与 encode](https://echarts.apache.org/handbook/en/concepts/dataset/)。打包版本来源和许可证见 [ECharts 资源说明](../Facetra/Resources/ECharts/README.md)。

## 多语言业务文案

`option` 的文字位置支持 `{i18n: key}`，从本图表顶层 `i18n.messages` 取值，随 App
语言切换更新标题、轴名称、图例和标签。给 `series` 提供稳定 `id`，避免将翻译后的
`name` 用作业务身份。普通字符串固定显示；数据绑定和脚本内容不会被自动翻译，翻译
结果也不会重新识别为 `js:`。内置语言同步使用 ECharts EN / ZH locale。完整语法、
回退及保存行为见 [多语言文案](localization.md#chart--dashboard-多语言文案)。
