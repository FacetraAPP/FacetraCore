# Charting Contract v1 发布前稳定性评估

当前产品尚未发布。原有冻结结论调整为：保留整体边界与已有文档语义，首次正式发布前
允许经审查扩展当前 v1，并同步更新模型、schema、兼容样例与显式基线；发布后才执行
下文所述 major 版本边界。这一决定覆盖本文原先针对新增 typed 字段必须升级 major 的约束。

## 发布前破坏性调整：Rust 渲染所有权

本轮按 Issue #85 将 YAML → 最终 HTML / ECharts / Vega / Vela / value 载荷的准备全部
移入 Rust。Swift / WebKit 只解码并展示 Rust 输出，不再把 DataFrame 映射为图形数据。
因此删除 Swift Charts 主机及 `default` renderer，并移除 `plot / x / y / average /
color / x_end` 字段；`chart.renderer` 改为必填。

这是明确授权的预发布破坏性调整，不提供兼容字段、隐式回退或自动迁移。读取旧字段的
文档会直接失败。模型、生成 schema、共享 valid/invalid 样例、`baseline.json`、Docs、
Gallery 和 UI 测试已同步更新；发布前仍沿用 v1 标识。

本轮将文案表 `i18n` 与 `{i18n: key}` 显式引用纳入 v1，支持 Chart / Dashboard 的
标题、说明、参数标签、分页标题和 ECharts option 文字。已有字符串文件无需迁移；
旧开发客户端需更新才能读取新语法。规则见 [多语言文案](../../../Docs/localization.md)。
基线仍防止无意漂移，不因尚未发布而自动刷新；本次指纹更新属于明确审查的发布前扩展。

参数新增可选 `width`，为有限且大于 0 的单个数值，单位为原生界面 pt，适用于 Chart
和 Dashboard。它覆盖整个「标题＋控件」参数块的自动宽度及自动宽度上限；窄屏时仍不
超过容器可用宽度。省略或 null 保留自动布局，不改变参数值、SQL 或绑定语义。旧文件
无需改写，旧开发客户端读取使用此字段的新文件前需升级。本次同步模型、schema、
共享兼容样例和显式基线，作为用户确认的发布前 contract 扩展。

本轮按用户请求新增 `html` 与 `vega_lite` renderer，作为发布前 v1 扩展。
现有 renderer、Dataset 必填/引用/SQL 规则保持兼容。新 renderer 只使用
`chart.option`，禁止 `chart.scripts`；HTML 仅接受 `html/css/data`，明确不接受
自定义 JavaScript。原生 Vega-Lite 配置保留其声明式表达式语义。已同步模型、
生成 schema、共享接受/拒绝样例和显式基线；旧客户端需升级后读取新 renderer。
详细行为见 [声明式渲染器](../../../Docs/declarative-renderers.md)。

以下保留架构评估及首次正式发布后的维护目标。

## 产品目标与设计判断

本评估以仓库 README、`Docs/design.md`、`Docs/data-pipeline.md`、CLI、发现页、
编辑与保存流程，以及 `facetra-contract`、`facetra-pipeline`、Swift adapter 的实际代码为依据。

Facetra 的核心是本机执行的数据可视化：用户或 AI 编写 YAML，数据通过 CSV、
HTTP、数据库、SQLite 进入 DataFrame，经 SQL 转换，再由 Rust 生成指标卡、ECharts
或 Vela 呈现。Dashboard 组合 Chart，Discover 和 Git 分享同一批可编辑文件。
因此首先需要长期保持文件可读、作用域明确、参数有类型、不同入口行为一致。

| 现有边界 | 与产品目标的关系 | 决定 |
| --- | --- | --- |
| `metadata / params / sources / dataset / chart` | 将身份、输入、取数、转换、呈现分开；已经覆盖仓库现有图表与模板 | 保留，不引入新一层文档包装 |
| Chart 拥有 source / dataset，按需导出引用 | 简单图表可以单文件分享，复杂图表可显式复用 | 保留；独立 Dataset/DataSource 资源等待实际产品需求 |
| Dashboard 共享 metadata，显式 panel 参数绑定 | 同一 Chart 可多次实例化，避免隐式全局参数影响结果 | 保留，不把 Dashboard 改为嵌入式 Chart 格式 |
| 按 metadata ID 引用，与文件夹无关 | 移动、分类和 Discover 导入不需要改变引用语义 | 保留；文件授权和依赖复制由宿主负责 |
| 纯 Rust contract，schema 从同一模型生成 | App 与渲染 CLI 使用同一规则，无需引入另一套格式实现 | 保留，不为假设中的 Web/WASM 客户端增加接口 |
| 结构 / 工作区 / 执行能力三层校验 | 无凭据、无网络时也能验证文件；renderer 与数据可用性仍能独立发展 | 保留，避免把 schema 通过等同于一定能执行 |
| 原始 YAML 与宿主状态不进入共享模型 | 编辑保留注释、原子保存与 revision 冲突检查，文件不携带设备路径或凭据 | 保留，不使用模型重序列化强制迁移文件 |

严格拒绝未知字段适合及早发现人工或 AI 生成的拼写错误，但意味着旧客户端不接受
新的 typed 字段。这是首次正式发布后冻结 v1、将后续格式扩展放入新 major 的主要原因。
开放 renderer option 仍须保留既有绑定含义；新增图形能力无需提前改变 Chart 顶层格式。

CLI 聚焦已授权工作区的检查、图表创作与执行，不访问 Discover 的目录、下载或导入
接口，也不通过 Discover 自动补齐缺失引用。现有 CLI 和 automation 命令均符合这个
边界，模板由 App 本地提供。通过 App 导入后的内容是普通工作区文件，可以继续由
CLI 使用。检查 pinned Discover 样例是验证共享 YAML 与 App 导入兼容性的开发测试，
不是 CLI 运行时的依赖。

## 冻结前发现并修复的问题

### HTTP 默认 JSON 与 recordsPath 不一致

`HttpSpec::validate` 曾在 `recordsPath` 非空时要求显式 `format: json`，虽然规范、
执行器和缺省行为都将省略或 null 的 format 解释为 JSON。结果是合法的嵌套 JSON
配置只因省略默认值而校验失败。

修复为按有效默认格式检查。共享样例 `valid/http-records-default.yaml`、Rust 中
省略/null/显式 JSON 的用例，以及 Swift 经本地 HTTPS fixture 实际获取嵌套记录的
测试覆盖这一行为。CSV 仍不能使用 recordsPath。

### 单个 source 复用被无关的必填参数阻塞

静态 Graph 已按 source 的实际依赖检查必填参数，但 Resolver 在进入目标 Chart
时调用完整参数解析。例如一个 Chart 声明尚未填写的筛选参数，其他 Chart 复用其
不依赖筛选的 HTTP source，静态校验成功，执行却报 Missing required parameter。

修复后，source 使用目标默认值和显式 with，只要求实际取数依赖存在并符合类型、
范围；整个 Chart 或引用整个 Dataset 仍要求完整必填参数。没有引入调用者同名参数
的隐式继承，也没有把缺失值转换成 null。

覆盖包括共享 `references/source-consumer.yaml`、Rust 中 HTTP/SQL、直接引用/本地
alias、目标默认值/显式参数、integer → number、缺失/越界输入，以及 Swift 从
同一共享样例经实际 Rust session 执行 HTTP 获取。现有动态选项 source 的独立加载
和依赖追踪继续保留。

两处问题都有修改前失败、修改后通过的 Rust 回归用例；无需修改现有 v1 schema
或重写工作区文件，属于使实现符合既有语义的修正。

## 首次正式发布后的 v1 维护承诺

- 冻结 typed 字段、必填项、枚举值、默认值、null/scalar 规则、引用语法和作用域。
  保存、导入或 App 升级不能要求用户重写已经合法的 v1 文档。
- Chart 保持现有无 version 的格式，Dashboard 保持 `version: 1` / `kind: Dashboard`。
  不在本轮给 Chart 补 version；未来新 major 必须定义显式识别和迁移方式，并继续读取 v1。
- 参数仍保持严格类型；required 无默认值是合法定义，执行时需要输入。
  单 source 只检查取数依赖，整个 Chart/Dataset 检查完整 required 集合。
- 保留文档中已承诺的 diagnostic code。调用者依据 code 定位与本地化；文本不作为协议。
  不承诺多个错误同时存在时的首错顺序。
- renderer 支持、真实列绑定、凭据、文件存在与授权、远端返回值属于执行能力。
  新增开放 option 能力不得悄悄改变原有绑定含义；切换渲染引擎仍需验证原有示例。
- typed 字段、参数类型、provider、renderer 的新增或既有语义改变进入新 major。
  性能、UI、存储、IPC 可以独立迭代，但不能破坏 YAML 边界。

## 防止后续无意漂移

`Tools/check-contract.sh` 现在包含两种独立检查：模型生成结果必须与提交的 schema
一致；生成结果还必须符合经审查的 `baseline.json`。只同时修改模型和生成产物不再
足以通过检查。基线忽略 JSON 空白和对象键顺序，生成器不会自动刷新基线。

schema 基线不能证明语义兼容，所以同时保留共享 valid/invalid/reference 样例与
`contract/tests/stability.rs`，约束默认值、null、YAML scalar/merge、引用参数
作用域及类型转换。Swift 继续运行同一 corpus，并验证实际模板和新增执行回归。
工作区实例检查覆盖 Docs/examples、UI fixtures，以及本地存在时的 pinned Discover。

每次修改 contract 或其消费者，运行 `Tools/check-contract.sh`；影响执行/adapter 时
再运行 `facetra-pipeline` 和 Swift release tests。发布还须完成项目已有的双端构建和 App/CLI
验收流程。本次行为修复的具体检查结果记录在 PR #60，不把一次测试结果当作未来
发布的永久证明。

当前仓库的 GitHub Actions 因额度问题仅允许 workflow_dispatch，本地检查接入了
core quality 流程，但还不能称为每个 PR 自动执行的合并门槛。恢复自动 CI 后应将
已有 quality job 设为必需检查；本次保留仓库暂停自动 workflow 的设置。

新增限制、重命名、改变默认值或作用域不能通过刷新基线作为 v1 发布。
仅纠正实现与既有规范的不一致时，才可以在 v1 修复，并附复现、兼容性说明和跨端
回归；若必须修正 schema，本身也需要明确审查基线差异。

## 后续不阻塞稳定版的事项

更精细的错误定位、更多边界样例、renderer 能力描述和查询性能可以继续完善。
只有出现明确需求时再评估独立数据资源、更多参数类型或离线/Web 客户端。
这些方向不构成现在继续改动 v1 格式的理由。

## 发布前扩展：Source dependency DAG

按用户请求在当前 v1 增加具体 source 的可选 `depends_on` 与 HTTP
`{{ sources.name.path }}` 模板。保留 params、ref/with 拥有者作用域、Dataset 输入
和现有无依赖文件的语义。同步生成 schema、显式基线、共享接受/拒绝样例，以及
Rust/Swift 的解析、真实 HTTPS、预览和异常回归。旧开发客户端读取新语法前需升级。
数组 fan-out、分页、重试不包含在本次 contract 中；执行规则与限制见 specification。

## 发布前扩展：HTTP collection

按用户授权在当前 PR 扩展 v1：状态驱动分页、受限 Rhai 表达式、逐项请求、HTML/CSS
与 XML/XPath 提取、采集限额及显式重试。同步 baseline、生成 schema、跨 Rust/Swift
接受/拒绝样例与真实 HTTPS/UI 回归。普通 HTTP 原始响应模板含义不变；新采集模式
返回合并记录数组。保留 DAG 无环、拥有者作用域、凭据隔离、取消与共享缓存边界。
旧开发客户端需升级。Cookie 会话、浏览器执行与历史采集不包含在本扩展。
