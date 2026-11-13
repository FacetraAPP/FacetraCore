# Vela OHLCV 蜡烛图

`renderer: vela` 使用离线打包的 [LuxAlgo Vela 0.8.4](https://github.com/LuxAlgo/Vela) 展示蜡烛图、成交量与技术指标。CSV、HTTP、数据库及共享 Dataset 沿用现有数据管线；Vela 只接收 SQL 结果，不自行连接行情服务。图表详情、看板、YAML 预览使用同一渲染器，支持明暗主题、十字光标、平移和缩放。

完整配方：[vela_ohlcv.yaml](examples/vela_ohlcv.yaml)。它引用已有的虚构 [OHLCV CSV](../FacetraUITests/Fixtures/ExampleWorkspace/data/example/echarts-ohlcv.csv)；复制到工作区对应的 `data/example/` 路径，或改成自己的 source。

```yaml
metadata:
  id: prices
  title: 日线
chart:
  dataset: candles
  renderer: vela
  option:
    time: trade_date
    open: open
    high: high
    low: low
    close: close
    volume: volume
    timeframe: 1d
    indicators:
      - type: sma
        inputs: {length: 20, color: '#f59e0b'}
      - type: rsi
        inputs: {length: 14}
```

## 数据约定

- `time/open/high/low/close` 必填，值为 `chart.dataset` 输出的列名。价格与成交量必须为有限的 SQL 数值，CSV 字符串请在 SQL 中 `CAST`。高低价必须包住开收盘价；成交量不能为负。NULL 或错误行会显示错误，不会静默丢弃。
- `volume` 可省略；有映射时默认显示成交量，`show_volume: false` 可隐藏。成交量默认叠加于价格图底部，使用独立刻度。
- `time_unit` 默认 `iso8601`，接受 `YYYY-MM-DD`、ISO 时间戳和 DuckDB `TIMESTAMP`；无时区值按 UTC 处理。数字时间必须声明 `seconds` 或 `milliseconds`，不猜测单位。结果统一为整毫秒时间戳。
- 适配器按时间升序排列。一个图只包含一个标的、一个周期，每个时间只能有一行；多标的先筛选，重复时间先在 SQL 中聚合。`timeframe` 默认 `1d`，支持正整数加 `m/h/d/w`（如 `15m`、`1h`）。它控制刻度和间距，不做重采样。
- 空结果显示「无匹配数据」。小高度卡片保留价格指标，隐藏成交量与振荡指标副图；完整图展示全部配置。窗口尺寸变化由 Vela 的 ResizeObserver 处理。
- 离线图默认只预留 2 根未来 K 线的位置，首次展示最近最多 200 根，随窗口变化重新适配宽度。用户平移、缩放或使用图内控件后保留其视角，不再自动重置。

## 指标扩展

当前支持 Vela 随包注册的内置指标，调用 `addNativeIndicator(type, {inputs})`，无需脚本引擎。每张图最多配置 16 项，同一指标可有不同周期的多个实例（`volume/vpvr` 等单实例指标除外）。指标类型、输入名称、输入类型、选项及数值范围在 WebKit 预览时依据当前 Vela 注册表检查，错误会明确显示。输入名与 Vela API 一致，区分大小写。

| 场景 | `type` | 常用 `inputs` | 位置 |
| --- | --- | --- | --- |
| 简单 / 指数均线 | `sma` / `ema` | `length`, `source: Close`, `color` | 主图 |
| 布林带 | `bollinger-bands` | `length`, `mult`, `basisColor`, `color` | 主图 |
| 相对强弱 | `rsi` | `length`, `source: Close`, `overbought`, `oversold` | 独立副图 |
| MACD | `macd` | `fastLength`, `slowLength`, `signalLength` | 独立副图 |
| 成交量 | `volume` | `upColor`, `downColor`, `heightPct` | 主图底部独立刻度 |

周期等输入可以绑定普通参数，应用参数后与图表一起重新计算：

```yaml
params:
  - {name: ma_length, type: integer, default: 20, minimum: 1, maximum: 5000}
# sources / dataset / chart 元数据省略
# chart.option 下：
indicators:
  - type: sma
    inputs: {length: {from: params, name: ma_length}}
  - type: sma
    inputs: {length: 60, color: '#3b82f6'}
```

指标使用本次 Dataset 返回的全部行计算；均线、RSI、MACD 有预热期，需要在 SQL 中保留足够历史，避免先截取展示窗口再计算。内置设置对话框的临时调整仅用于当前视图，持久化配置应写入 YAML。

后续扩展建议按以下顺序推进：

1. **指标选择与配置界面**：使用 Vela 的 `availableNativeIndicators()` 和输入 schema 生成选择器、周期与颜色编辑器，保存回上述 YAML；支持显示 / 隐藏、排序、显式副图位置及指标预设。继续让 YAML 成为持久化来源。
2. **SQL 自定义因子**：在 Dataset 中计算复权价格、收益率、行业基准和业务因子，再增加按时间绑定数值列的通用线 / 柱适配。为 NULL 预热值、独立刻度和多副图定义协议，共享时间轴。当前版本尚不将额外 SQL 列绘成 Vela 指标。
3. **脚本与插件**：确有复杂指标需求时再引入 Vela 的 ScriptingEngine / Plugin API。单独评估执行超时、后台计算、结果缓存与许可证；PineTS 插件采用 AGPL，不随当前 Apache-2.0 核心打包。预警、策略回测应建立独立的数据与执行协议。

资源来源、校验和及许可证见 [Vela 资源说明](../Facetra/Resources/Vela/Vela-README.md)。图表保留上游可见归属标记，链接通过系统浏览器打开。

实际应用中的详情、看板、参数及 Discover 导入验证见 [Xcode UI 验证记录](vela-ui-verification.md)。
