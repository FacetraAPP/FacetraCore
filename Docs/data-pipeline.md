# YAML、连接配置与数据管线

Facetra 参考 Spectra 的 `content/charts/example/example.yaml`、`backend/app/data/render.py`、`backend/app/compiler/references.py` 和编译入口。采用 source → Dataset → Chart 的边界，以及拥有者作用域、目标默认值和显式 `ref.with`。Facetra 保留带类型的参数定义和结构化绑定，并由 Rust 为所有 renderer 生成最终载荷；本轮不执行 Jinja 控制流、Python 或 provider 专用代码。

## 定义与执行

```text
Keychain（应用级连接配置值，经连接自身的别名解析）
   ↓ HTTP 按字段引用读取；数据库按连接别名读取
params + sources → source DataFrame → dataset SQL → DataFrame → chart → Dashboard
```

Chart 文件只有 `metadata`、`params`、`sources`、`dataset`、`chart` 五类顶层字段。ID、标题、说明和标签属于 `metadata`；文件夹由 `charts/` 中实际相对路径决定，不写入 YAML。一个 Chart 默认拥有自己的 source 与 Dataset；需要复用时引用其他 Chart 导出的定义，不创建独立 Dataset 或 DataSource 资源。配置文件名必须是 `<metadata.id>.yaml`，可以位于 `charts/` 的任意多层子目录。资源 ID 在整个空间内唯一，跨图表引用仍使用 `<id>.yaml#/sources/<name>` 或 `<id>.yaml#/dataset`，按 ID 在目录树中解析，移动文件保持引用有效。

```yaml
params:
  - { name: symbol, type: string, default: "600519.SH" }
sources:
  prices:
    source: duckdb
    file: data/example/stock-daily.csv
dataset:
  id: prices
  sources:
    - { id: raw, ref: "#/sources/prices" }
  sql: |
    SELECT CAST(trade_date AS DATE) AS trade_date, CAST(close AS DOUBLE) AS close
    FROM raw WHERE symbol = $symbol ORDER BY trade_date
metadata:
  id: stock_price
  title: 股价
chart:
  dataset: prices
  renderer: echarts
  option:
    tooltip: {trigger: axis}
    xAxis: {type: category, data: {from: prices, field: trade_date}}
    yAxis: {type: value, scale: true}
    series:
      - type: line
        name: close
        showSymbol: false
        data: {from: prices, field: close}
```

`params` 默认空列表，`sources` 默认空映射；`dataset` 和 `chart` 必填。参数需要 `name/type`，可配置 `label/default/required/minimum/maximum/width/options/advanced`。省略 label 使用 name；required 默认 true，advanced 默认 false。当前资源保存和预览需要能够用默认参数执行。输入类型严格校验，字符串 `"30"` 不会变成整数。

`width` 是可选的单个正数，单位为原生界面 pt，控制整个「上方标题＋下方输入控件」参数块的宽度；例如 `width: 120`。设置后优先于自动测量和自动宽度上限；屏幕过窄时最多使用容器可用宽度。省略或设为 null 时自动布局。Chart 和 Dashboard 参数都支持；它不影响参数值及查询执行。

`options` 可以是原有的静态值列表，也可以引用现有 source。动态选项不需要加入 `dataset.sources`：

```yaml
params:
  - name: symbol
    type: string
    default: "600519.SH"
    options:
      ref: '#/sources/symbols'
      value: code
      label: name
sources:
  symbols:
    source: database
    connection: market
    sql: SELECT code, name FROM symbols ORDER BY code
```

`value` 是参数值列，默认 `value`；`label` 是可选的显示列，省略时显示参数值。值严格匹配参数类型及范围，NULL 值跳过，重复值保留第一次出现的行，行顺序不变。名称为 NULL 时显示参数值。缺少列或类型不匹配显示加载错误；空列表显示「没有可用选项」。HTTP JSON 还支持同类型标量数组（如 `["A", "B"]`、`[1, 2.5]`、`[true, false]`，可含 NULL），统一转为 `value` 列；空数组转为空表。混合不同标量类型的数组仍报错。

引用支持原有的 `#/sources/name`、`chart_id.yaml#/sources/name` 和 `with` 参数绑定。跨文件引用仅使用目标默认值和显式传参；source 加载只要求实际用到的参数存在，不依赖其他必填参数已经选择。选项 source 不得依赖当前 dropdown 自身的值。看板参数使用跨文件 source 引用，例如 `options: {ref: 'catalog.yaml#/sources/symbols', value: code, label: name}`；看板不增加自己的 `sources` 映射。保存 YAML 校验结构和引用，不获取远程选项。

选项与图表、原始数据预览共用 source 缓存。CSV 不设 TTL；HTTP、数据库和 SQLite 的 TTL 为 60 秒，在下一次访问时检查。打开动态 dropdown、依赖参数变化及手动刷新时重新经过同一加载入口；source 定义、空间、刷新代次和连接版本参与缓存身份。TTL 不会主动推送更新，也不持久化到磁盘。失败可重试，旧请求取消后不会覆盖新选项。刷新列表保留已选值；值已被移除时提示重新选择，Apply 使用当前 source 缓存中的选项检查值。

静态与动态 dropdown 在去重后超过 10 项时提供搜索，搜索同时匹配显示名称和实际值；搜索只过滤已加载列表，不触发请求。macOS 使用 popover，iOS 使用 sheet。选择仍只修改草稿，点击 Apply 后才更新图表。

Source 定义获取数据的方式。`source: duckdb` 使用 `file` 读取当前空间 `data/` 内的 CSV，DuckDB `read_csv` 保留全部原始字符串，避免代码丢失前导零。业务类型转换在 SQL 中显式 cast。系统导入 CSV 文件或文件夹会复制文件，为每个 CSV 生成一个 source；最多 32 个，单文件 100 MB / 100,000 行，只扫描目录第一层，拒绝符号链接与名称冲突。

图表详情默认显示图表，点击工具栏的「表格」按钮可切换为原始数据，再点击「图表」返回。详情卡片不显示折线图、柱状图等类型标识。多个输入可按 Dataset 的 source ID 切换；这里只展示输入数据，不展示文件路径、连接、URL 或 SQL 信息。Notes 默认折叠，可独立展开。

数据视图将源头选择器、表格行数与刷新按钮放在同一行；源头列表可滚动，长名称换行，选中标记位于右侧。刷新同一源头时保留当前数据，在按钮处显示加载状态，失败后保留数据并允许重试；切换源头或输入内容变化时清除旧预览。表格列宽根据字段和部分数据调整，点击单元格可查看及复制完整内容。JSON 支持语法高亮与节点折叠；JSON/text 的复制按钮始终复制完整内容，与折叠状态无关。

每个 source 可通过 `display: df | json | text` 指定原始数据的展示方式，省略时使用 `df`。这与 HTTP 的 `format: json | csv`（响应解析格式）独立，不改变图表的 Dataset SQL 或绘制逻辑。

```yaml
sources:
  prices: {source: duckdb, file: data/prices.csv} # 默认 df
  response:
    source: http
    url: https://example.com/api/prices
    format: json
    recordsPath: [items]
    display: json
  csv_text: {source: duckdb, file: data/notes.csv, display: text}
  shared: {ref: 'example_price.yaml#/sources/prices', display: text}
```

`df` 尝试将输入转换为表格，转换失败时回退到原始 text。`json` 直接显示完整 JSON 响应（包含 recordsPath 之外的内容），`text` 直接显示响应或 CSV 文件原文；它们无需先成功转换为 DF。CSV、数据库和 SQLite 的表格数据选择 `json` 时转换为记录对象数组，表格源选择 `text` 时转换为 CSV 文本。无效 JSON 回退到 text。source 引用继承目标的 display，也可在当前引用上覆盖；界面仅在用户打开原始数据时按需获取所选源头。

Dataset 的 `sources[].id` 是 SQL 中的表名，`ref` 选择实际输入。多 source 可以跨文件关联，不要求来自同一个文件夹。SQL 接受单条 SELECT / WITH，支持 join、聚合和窗口；`$name` 通过驱动绑定为有效 Dataset 参数，不进行字符串插值。一个输入可以省略 SQL 并直接返回 DF；多输入需要 SQL。没有隐式主表，创建表单会显式将选中的主表命名为 `raw`。

DF 包含有类型的列、行和 null。HTTP JSON 将对象数组作为多行、单个对象作为一行 DF，保留 boolean/integer/number/null，支持 `recordsPath` 指定嵌套记录对象或数组；异构列转为字符串，嵌套对象 / 数组保留 JSON 文本。SQL 输出保留日期和时间戳，Decimal 和超宽整数以文本返回。Rust 按 SQL 行顺序读取输出列并生成最终渲染载荷，不在 Swift 中重新排序、筛选或计算均线。`renderer: echarts` 使用同一完整 DataFrame，以 `chart.option` 的递归绑定配置展示；具体绑定、参数、脚本和离线示例见 [ECharts YAML 指南](echarts.md)。value renderer 使用 chart.value 指定字段，展示单行 Dataset 的字符串、数字或布尔值，null 显示为 —，多行结果必须先在 SQL 中聚合或筛选。

`renderer: vela` 将同一 SQL 结果按 `chart.option.time/open/high/low/close/volume` 映射为 OHLCV，按时间排序并拒绝重复时间。内置技术指标基于这些行情行计算，输入可绑定参数；时间单位、成交量及主副图配置见 [Vela OHLCV 指南](vela.md)。

## 引用与作用域

引用可以直接出现在 `dataset.sources[]`，也可以将复用定义导出到当前 `sources` 映射：

```yaml
sources:
  prices:
    ref: "example_price.yaml#/sources/prices"
    with:
      symbol: { param: symbol }
dataset:
  id: prices
  sources:
    - { id: raw, ref: "#/sources/prices" }
  sql: SELECT * FROM raw
```

也可以复用整个 Dataset：

```yaml
dataset:
  id: shared_prices
  ref: "example_price.yaml#/dataset"
  with:
    symbol: { param: symbol }
    days: 30
```

本地引用继承当前有效参数；跨文件引用只接受目标默认值和 `with` 显式传入值，不隐式继承调用者的同名参数。`{ param: name }` 在调用者作用域中解析，字面值保持类型。目标 Dataset 的 SQL 和 source 请求在目标作用域执行，当前 Chart 的显示配置保持独立。ref 定义不能叠加 source 或 SQL 字段。

引用只支持当前空间的 `<id>.yaml#/sources/<name>` 和 `<id>.yaml#/dataset`，按 ID 解析 `charts/` 中的图表；本地写 `#/sources/<name>`。引用字符串不包含 `charts/` 前缀。不接受 `./`、`../`、绝对路径、跨空间或任意 JSON Pointer。循环和超过 64 层引用会报错。编辑器中的未保存定义优先于磁盘副本。保存会验证受影响图表，看板与图表引用会阻止删除仍被使用的 Chart。

完整可执行示例在 [examples](examples/)，包含独立 SQL 转换与共享 Dataset 两种复用。将 Chart YAML 放到空间的 `charts/`，Dashboard YAML 放到 `dashboards/`，将应用示例 `stock-daily.csv`、`symbols.csv` 复制到 `data/example/` 即可使用本地示例。

## HTTP 与连接配置

设置中的「连接库」支持 Key-Value、PostgreSQL、MySQL，选择器显示为 Type（连接类型）。Key-Value 是通用键值配置：新建时没有预设字段，点击 Add（添加）手动填写 Key 和 Value，支持修改键名和删除字段，不显示 Extras。Notes（备注）可选，标签与全宽输入区分开显示，输入区随自动换行或手动换行增加高度，完整显示内容。

| 连接类型 | 字段 | 默认值 |
| --- | --- | --- |
| Key-Value | 用户手动添加的 Key / Value | 无预设字段 |
| PostgreSQL | host、port、schema（数据库）、login、password、extra | port 为 5432 |
| MySQL | host、port、schema（数据库）、login、password、extra | port 为 3306 |

数据库 host、schema、login 必填，password 可选。数据库类型沿用固定字段，Extras 对应配置键 `extra`，默认留空；空白表示没有附加选项，填写时必须为不超过 64 KB 的 JSON 对象，可保存嵌套附加配置。连接 ID 和已保存的类型固定。Key-Value 沿用 `connector: http` 的持久化标识，自定义键名须唯一，并遵循 ID 的 1–80 字符规则，可通过 `{{ connections.market.my_key }}` 引用。

全局连接元数据以 Application Support 下的 `Facetra/Connections/<id>.yaml` 原子保存，字段为 `id/name/connector/notes/aliases` 和可选 `workspaceType`，Key-Value 另存 `fields[].key`。配置值以 JSON 字典保存在系统 Keychain 的 `Facetra.GlobalConnectionValues` 服务（WhenUnlockedThisDeviceOnly），account 为全局连接 ID。列表不读取配置值。连接 ID 自动生成且保持固定；名称、字段与别名可编辑。macOS / iOS 均从「设置 → 连接库」创建或编辑连接；表单中的「别名」区块默认显示一行，至少填写一个别名；支持逐行添加、删除，最后一行保留。每行别名可选择全局或指定 workspace，作用域保存本机 workspace 稳定 ID，`workspaceName` 仅作显示缓存。

同一连接可以有多个别名，每个别名独立选择作用域；可对不同 workspace 配置同名别名。图表通过 `{{ connections.market.endpoint }}` 或 `connection: market` 使用任一别名，也可直接使用连接 ID。别名遵循 ID 的 1–80 字符规则，按 `(workspace ID, alias)` 校验唯一，全局别名在全局作用域唯一，不能与其他连接的 ID 重复。执行先匹配当前 workspace 的别名，再匹配全局别名，最后支持连接 ID。删除连接或移除别名前，检查所有已注册空间（含卸载空间）的图表引用；按各 workspace 实际解析结果判断依赖，有效引用仍在使用时拒绝删除；类型不符合声明的别名可移除或改名，以修复绑定或恢复全局回退。删除或重置空间保留全局连接、别名和配置值。查询仅投影实际引用的连接值、绑定 ID 和语义版本。修改相关配置值或绑定会使该查询缓存失效，并丢弃旧响应；修改显示名称或无关连接不重新获取 source。

workspace 的 `connections/<defined-type>.yaml` 是 Key-Value 类型的界面包装，包含可选 `name`、`notes` 和必填的 `fields: [{key: token}]`。Type 选择器显示 `类型名<workspace 名>`，来源由本机 workspace 稳定 ID 和定义文件名标识。全局连接 YAML 的 `workspaceType` 保存 `typeID/typeName/workspaceID/workspaceName`（兼容迁移旧名称身份）；`connector` 仍为 `http`，值仍是原有 Keychain 字典。

有定义时按定义展示字段，已有额外键值保留。workspace 卸载、删除，或定义缺失、无效时，连接界面显示「不支持的类型」并退回普通 Key-Value 编辑，仍可编辑、通过别名引用和查询；定义的可用状态不参与查询解析；workspace 清单不再声明 `requiredConnections`；图表按实际使用的 dataset/source、参数选项 source 和跨图表引用推断连接别名。HTTP 从 `connections.alias.key` 模板推断，数据库从 `connection: alias` 推断，所选连接仍由 source 执行器校验。旧清单中的声明读取时忽略，不改写原文件。缺少连接不阻止编辑，实际使用它的请求须先完成配置。重新加载原 workspace 可恢复定义；删除后新建同名工作区具有新的稳定 ID，不接管旧类型来源或连接作用域。只接受此最终格式，不迁移旧 workspace 连接或旧 Keychain 元数据。另一设备仍需填写自己的连接值。

元数据示例：

```yaml
id: 9e6d2a90-9a10-4ef4-b733-1a647ddcb912
name: 市场行情
connector: http
aliases:
  - alias: market
  - alias: research
    workspace: 84cf5a35-bf22-4d65-965c-60106b676f1d
    workspaceName: Research
fields:
  - key: endpoint
  - key: token
notes: |
  每日行情查询，只读使用。
```

图表中的引用示例：

```yaml
source: http
method: post
url: "{{ connections.market.endpoint }}"
params:
  apikey: "{{ connections.market.api_key }}"
headers:
  Authorization: "Bearer {{ connections.market.token }}"
json:
  symbol: "{{ params.symbol }}"
  options:
    limit: "{{ params.limit }}"
format: json
recordsPath: [data, items]
```

HTTP 默认 GET，支持 GET / POST；`params` 是 query，`headers` 是请求头，`json` 是可嵌套 JSON 请求体，`data` 是 form 请求体。json/data 互斥，GET 不接受请求体。format 默认 json，可选 csv；recordsPath 仅用于 JSON。URL 允许已有 query，额外 params 通过 URLComponents 编码；表单值也进行编码。

模板支持 `{{ params.name }}`、`{{ connections.id.key }}`。`type: date` 参数的输入、默认值和折叠摘要统一使用 ISO 日期格式 `YYYY-MM-DD`，支持手动输入或原生日历选择，应用时校验日期有效性；SQL 和未加过滤器的请求保留该格式。如果接口需要其他格式，在 YAML 的 HTTP source 中显式使用 `{{ params.start_date | date_format('yyyyMMdd') }}`。日期过滤器只用于参数引用，支持 `yyyyMMdd` 和 `yyyy-MM-dd` 输出，不修改保存的参数值。完整占位符作为 JSON 值时保留参数的整数、数值、布尔类型；字符串内拼接产生字符串。Keychain 值始终是字符串，渲染结果不会再次作为模板执行。URL、query 值、header 值、JSON / form 值可引用配置，配置既可保存 secret，也可保存完整 endpoint 或路径片段。编译及缓存键不读取明文，HTTP 执行时才查询 Keychain。缺少配置直接报连接 ID 和字段 Key，不输出配置值、完整请求或服务端响应正文。

请求要求 HTTPS，不接受 URL 用户名、密码或 fragment，重定向仅允许同源 HTTPS。读取超时 30 秒，响应上限 100 MB；CSV / JSON 最多 100,000 行。修改或删除连接配置、手动刷新数据都会增加查询代次，使旧缓存失效。source 执行支持 DuckDB CSV、HTTP、PostgreSQL、MySQL 和 SQLite，均由 Rust 执行。

Apple 平台的 HTTP 通过 native-tls / Security.framework 验证系统证书信任；其他平台使用 rustls 的本机根证书。请求头名称按 HTTP token 规则校验，支持 `FTSHARE_API_KEY` 这类含下划线的名称；非法名称、换行和控制字符仍拒绝，传输错误不暴露 URL 或凭据。

额外信任锚由宿主通过 `context.http_roots` 传入，不属于图表 YAML。PEM bundle 在 Rust 中解码后逐张以 DER 导入，避免 native-tls 的 PEM 导入在 iOS 上触发未实现的 panic；空 bundle 和 PEM 解码失败返回 `Invalid host certificate`。隔离 UI 测试使用满足 Apple 有效期和 `serverAuth` 要求的本地证书，仍校验服务器证书和主机名。

## 原生数据库查询

`source: database` 通过全局连接的别名使用 Rust 数据库驱动 3.36.0 的统一 `SQLDatabase` 接口查询，支持 macOS / iOS。连接工厂根据 connector 构造会话；PostgresKit 2.17.0 适配 PostgresNIO，MyRust 数据库驱动 4.10.1 和 SQLiteKit 4.5.2 提供对应方言与参数编码。在图表 YAML 中填写连接别名和 SQL；source SQL 使用目标数据库方言，Dataset SQL 仍使用 DuckDB。

```yaml
params:
  - { name: minimum, label: 最低金额, type: number, default: 100 }
sources:
  orders:
    source: database
    connection: business
    sql: |
      SELECT created_at, amount FROM orders WHERE amount >= $minimum
```

查询编译器将 SQL 文本与 `$name` 参数分开，生成 `SQLExpression`；Rust 数据库驱动 根据当前连接方言生成 `$1`、`?` 或 `?1` 占位符，同一执行器负责流式转换 DataFrame、结果限制和超时。SQLite 的 `SQLDatabase` 适配器使用系统 SQLite3，以保留只读打开、进度回调取消与精确 UTF-8 文本；MySQL 使用可取消的 prepared command，避免底层驱动在中断查询时的 statement 断言。

Source 接受 `connection` 和 `sql`，不在 YAML 中放置驱动、地址或凭据。SQL 限制为单条 SELECT / WITH，参数值通过驱动绑定，重复参数按出现顺序绑定；支持标准字符串/标识符引号和 SQL 注释，不支持原始 `?` 占位符、分号、美元引号、反斜线字符串转义或 `#` 注释。可复用现有 source / dataset refs 和 `with` 参数作用域。

SQLite 使用独立的文件 source：

```yaml
source: sqlite
file_path: databases/orders.sqlite # 对应 workspace/data/databases/orders.sqlite
sql: SELECT created_at, amount FROM orders WHERE amount >= $minimum
```

`file_path` 相对于当前空间的 `data/`，不得使用绝对路径、隐藏路径、空路径段或 `..`。SQLite source 不接受 connection、file 或凭据。图表页顶部「导入文件」可选择 CSV / SQLite 文件，填写文件名和 data/ 下的目标子目录；读取输入前和写入前均检查目标路径，已有文件不覆盖。SQLite 使用 backup API 导入包含已提交 WAL 页的一致快照，最多 100 MB；不会与外部原文件自动同步。查询只读打开已有文件，保留参数绑定、取消和超时（查询 30 秒，锁等待最多 100 毫秒）及结果限制。PostgreSQL/MySQL 每次未命中缓存的查询新建连接，在只读事务中执行，完成、失败、超时或取消后关闭；当前没有连接池。查询结果最多 100,000 行 / 100 MB，NULL 保持缺失值，精确 decimal 保持字符串，二进制值使用 Base64；后续 SQL 可显式 cast。

数据库 Extra 支持：

| 选项 | 适用类型 | 行为 |
| --- | --- | --- |
| tls_mode | PostgreSQL / MySQL | 默认 `verify-full`：TLS 并验证证书；`require`：TLS、不验证证书；`disable`：关闭 TLS |
| server_name | PostgreSQL / MySQL | 指定 TLS 证书主机名；默认使用 host |
| connect_timeout | PostgreSQL / MySQL | 连接超时，默认 30 秒 |
| query_timeout | PostgreSQL / MySQL | 查询超时，默认 30 秒 |

超时选项为 1–120 的整数秒。其他 extra 键可以保存，驱动只应用上表中的选项，不将任意 extra 拼入 SQL。MySQL 驱动协商 TLS 后检查会话，要求 TLS 时拒绝未加密会话。原始驱动错误和调试日志不显示查询、响应或凭据。远程查询无记录时提示检查查询与参数；SQLite 保留空结果的字段。

## 已移除的数据源

`source: healthkit` 已不再支持，健康图表与看板也已移除。应用不再申请健康权限。旧 YAML 显示明确的错误，原文件保留；可替换为 CSV、HTTP 或数据库来源。

## 存储、缓存与隔离

Rust 负责所有 source 的获取、请求模板、参数绑定、解析、原始数据预览及 Dataset SQL。Swift 负责文件选择与 security-scoped access、Keychain 和界面任务；Rust 仅接收当前查询所需的连接字段以及不含明文凭据的版本标识。

会话保留一份 DuckDB 1.4.4 数据快照与最近的查询结果。SQL 参数变化复用已导入的表；来源、空间、刷新代次或远程 TTL 变化重建快照。最多 32 个 source、100 MB 数据预算、100,000 行结果；每个 DuckDB 引擎内存上限 128 MB，禁止扩展自动加载和落盘溢写。CSV 通过目录句柄逐级打开并拒绝符号链接，私有临时快照导入后永久关闭外部访问，再执行用户的 Dataset SQL。

Source 缓存及并发请求合并由 Rust 管理。HTTP 和数据库结果有效期 60 秒，本地 CSV 持续到实际文件元数据、刷新代次或注入内容变化。多个调用共用获取过程，取消一个等待者不影响其他调用；最后一个等待者取消时关闭获取任务并等到文件句柄释放。错误不会永久污染缓存。

Rust 返回统一的 typed DataFrame，保留列顺序、类型、NULL 和行顺序。所有 renderer 的 option/scripts、图表数据和 i18n 最终结构由 Rust 统一编译，应用只接收最终 JSON 或数值载荷，避免完整 DF 往返与重复映射。修改图表样式不重新获取数据或执行 SQL。表格预览首次最多传输 256 行，后续按同一冻结快照逐页加载；原文/JSON 预览仍保持原有内容。协议、取消与快照释放见 [Source 接口](../core/SOURCE_INTERFACE.md)。

保存保留编辑原文的注释与排版，按原始字节检查冲突；结构、引用和可执行本地查询通过后才原子写入。无效文件隔离显示，不阻止无关资源使用；修复后恢复依赖。示例 YAML/CSV 只属于测试资源；UI 测试运行时写入隔离目录并在结束时清理，应用包不包含这些文件。发现页安装的资源是可编辑的普通文件。

旧 DataSource、旧 Chart 和独立 Dataset 格式全部移除，不提供兼容或自动迁移。仓库可执行示例、Schema、测试和内置内容统一使用当前协议。Dashboard 继续使用 `version: 1 / kind: Dashboard`，实例参数显式绑定共享参数；Focus 修改不回写看板。

## 重写旧配置

迁移已有文件时，先将每张 Chart 改写为当前五块结构，再移除独立 DataSource 文件；应用只加载新格式。

| 原定义 | 当前写法 |
| --- | --- |
| Chart 顶层 id/title/notes、chart_type | 放到 metadata.id/title/notes、chart.renderer |
| 本地 DataSource file / folder.tables | 每个 CSV 写成 sources.<name> 的 source: duckdb 与 data/ 相对路径；本地 JSON 先转换为 CSV |
| HTTPS DataSource 与 credential | 写成 source: http，请求字段内显式引用连接配置；在当前空间连接配置中重新保存值 |
| 单字段连接与 keychain.name 模板 | 在连接配置中创建名称、Key-Value 类型与自定义键值字段，将模板改为 connections.id.key；旧 Keychain 项不读取或自动导入，需在新界面重新保存值 |
| dataset.source / table | dataset.id 与 sources[].id/ref；SQL 使用声明的表名，主表 raw 也要显式声明 |
| 多图共用 DataSource | 一个 Chart 拥有 source，其他 Chart 使用 <id>.yaml#/sources/<name> |

Dashboard 的 Chart ID 和实例参数绑定保持不变。仓库示例及安装模板已经完成这些改写，不保留旧定义文件。

## HTTP 示例：IPv4 分段饼图

[HTTP 饼图示例](examples/http_ip_pie.yaml) 从 `connections.http_example.endpoint` 读取 URL，可在共享连接库配置 `http_example` 别名和 `endpoint: https://api.ipify.org`。请求通过 `params: { format: json }` 获取 [ipify 的 IPv4 JSON 响应](https://www.ipify.org/)。安装该 YAML 后，需先创建包含 `endpoint` 字段的共享连接，并为该连接添加 `http_example` 别名；应用不再提供隐式的示例连接。

响应对象的 `ip` 字段通过 DuckDB SQL 拆成四行，每行包含 `segment` 标签与 `value` 数字。`198.51.100.42` 对应 `[198, 51, 100, 42]`，饼图扇区按四段数值之和计算占比。零值占比为零，仍出现在图例中；不接受 IPv6、格式错误或超出 0–255 的分段。

饼图使用 `renderer: echarts` 与 `series[].type: pie`；DuckDB 输出的 `segment` 作为分类标签，`value` 作为扇区数值。Rust 要求数值非负且有限、至少有一个正值。导入该配方后可在「图表 → HTTP 示例 · IP 分段」打开，HTTP 失败会显示错误，可用数据表中的刷新按钮重试；「编辑 YAML」可修改定义。

```yaml
sources:
  connection:
    source: http
    url: "{{ connections.http_example.endpoint }}"
    params: { format: json }
    format: json
```

```yaml
metadata:
  id: http_ip_pie
  title: HTTP 示例 · IP 分段
chart:
  dataset: network
  renderer: echarts
  option:
    tooltip: {trigger: item}
    series:
      - type: pie
        radius: 65%
        encode: {itemName: segment, value: value}
        data: {from: network, rows: true}
```

## 数据库测试

`swift test --enable-swift-testing` 验证 SQLite 真实文件、Rust 数据库驱动 方言序列化和 query builder、工作区连接类型包装与退化，以及使用 `Tools/test-database-server.py` 回环协议夹具运行的 Postgres/MySQL 驱动查询、绑定、超时与取消。夹具不需要安装数据库。

真实 Postgres 集成测试需显式设置 `FACETRA_LIVE_POSTGRES=1` 和 `FACETRA_TEST_PG_HOST`、`FACETRA_TEST_PG_DATABASE`、`FACETRA_TEST_PG_USER`、`FACETRA_TEST_PG_PASSWORD`，数据库名称必须以 `_test` 结尾，再运行 `swift test --filter livePostgresSourceWithSeededData`。它创建一张唯一命名的 `facetra_source_test_*` 表和 3 条记录，验证参数查询、Rust DuckDB 汇总与最终 ECharts 载荷，并在结束后删除测试表；设置 `FACETRA_TEST_PG_TABLE` 可复用已有夹具表，避免再次写入。`FACETRA_TEST_PG_TLS` 控制测试连接的 TLS 模式。凭据只从环境读取，不写入仓库或测试日志。

## Dataset 结果与声明式渲染器

图表的表格入口在原有选择器中增加「Dataset 结果」，与下方「数据源」分组区分。
Dataset 结果始终以表格展示实际 SQL 输出；源头的 `display: json | text` 不影响结果表格。
刷新保留旧数据直至成功，查询或选中项变化时清除旧结果，失败可重试。

`renderer: html` 与 `renderer: vega_lite` 使用同一 Dataset 和绑定；HTML 采用
HTML/CSS 与声明式模板，不支持自定义 JavaScript。Chart/Dashboard Notes 支持 Markdown
及 `mermaid` 围栏代码块。完整协议与示例见[声明式渲染器](declarative-renderers.md)。

## Source 串联请求与 DAG

后续 [HTTP 分页与网页采集](http-collection.md) 扩展支持通用状态驱动分页、受限表达式、
逐项请求、HTML/CSS 与 XML/XPath 提取，以及显式限额和重试。下文无分页说明描述
原始 DAG 行为；新设置的执行和输出语义以扩展协议为准。

完整的 [公开 API 天气示例与 UI 测试](../FacetraUITests/Fixtures/WeatherWorkspace/README.md)
演示 ifconfig.me 获取 IP、ipwho.is 获取近似位置、Open-Meteo 获取当前摄氏温度。
同一 YAML 包含默认本地 HTTPS 回归和可选的 macOS / iOS 真实公网 UI 测试。

在具体 source 上声明 `depends_on`，再在 HTTP 请求模板中读取直接上游的结果。
跨图表先声明 `ref` 别名；依赖始终在 source 所属图表的参数作用域中解析。

```yaml
sources:
  login:
    source: http
    method: post
    url: https://example.com/login
    json: {token: '{{ connections.api.token }}'}
  items:
    source: http
    depends_on: [login]
    url: https://example.com/items
    headers: {Authorization: 'Bearer {{ sources.login.access_token }}'}
  detail:
    source: http
    depends_on: [items]
    url: https://example.com/detail
    params: {id: '{{ sources.items.0.id }}'}
dataset:
  id: data
  sources: [{id: raw, ref: '#/sources/detail'}]
```

依赖必须先成功；缺失节点、重复依赖、环和未声明的上游引用会被拒绝。
HTTP JSON 读取原始响应，不受 `recordsPath` / `display` 影响；CSV 和数据库读取
记录数组。路径用点号，数字段选择数组元素；整值 JSON 模板保留对象、数组、布尔值
与 null。空数组越界、缺失字段和无效 JSON 会报错，不自动填空或跳过。

正式 Dataset、source 预览和动态参数选项共用依赖执行与缓存；共享上游只获取一次，
上游 acquisition 版本变化会失效下游缓存。失败和取消不继续下游，后续可重新发起。
目前不自动展开数组、分页或重试；无依赖的分支不承诺并发，依赖列表顺序不代表执行顺序。
每次解析最多展开 4096 个节点，深度沿用 64 层上限；单 source 的依赖取数总量上限为
100 MiB（共享 acquisition 版本只计一次）。完整规则见 [contract](../Contracts/charting/v1/specification.md#source-dependency-dag-pre-release-v1-extension)。
