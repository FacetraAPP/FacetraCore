/* Generic ECharts host. Data and script locations arrive separately from Swift. */
(() => {
  const container = document.getElementById("chart");
  let chart;
  let theme;
  let locale;
  let renderID = 0;
  const report = (status, message) => window.webkit.messageHandlers.chartStatus.postMessage({ status, message, id: renderID });

  function compile(payload) {
    const option = payload.option;
    for (const script of payload.scripts) {
      let target = option;
      for (const key of script.path.slice(0, -1)) target = target[key];
      const key = script.path[script.path.length - 1];
      Object.defineProperty(target, key, {
        value: (0, eval)(`(${script.source})`),
        enumerable: true, writable: true, configurable: true,
      });
    }
    return option;
  }

  const components = (value, apply) => value === undefined ? undefined : Array.isArray(value) ? value.map(apply) : apply(value);
  const styled = (value, defaults) => ({ ...defaults, ...value, textStyle: { ...defaults.textStyle, ...value?.textStyle } });
  function viewDefaults(authored, small, compact) {
    const size = compact ? 9 : small ? 10 : 12;
    const axis = (value, name) => ({
      ...value,
      name: compact ? "" : value.name,
      splitNumber: value.splitNumber ?? (small ? 3 : 5),
      nameTextStyle: { fontSize: size, ...value.nameTextStyle },
      axisLabel: { hideOverlap: true, fontSize: size, margin: compact ? 3 : 6, ...(value.type === "category" ? { interval: "auto" } : {}),
        ...(name === "xAxis" ? { alignMinLabel: "left", alignMaxLabel: "right" } : {}), ...value.axisLabel },
    });
    const result = {
      textStyle: { fontSize: size, ...authored.textStyle },
      tooltip: components(authored.tooltip, (value) => styled(value, { confine: true, triggerOn: small ? "click" : "mousemove|click", textStyle: { fontSize: size + 1 } })),
      legend: components(authored.legend, (value) => ({
        ...styled(value, { type: "scroll", itemWidth: small ? 14 : 20, itemHeight: 8, itemGap: small ? 8 : 12, textStyle: { fontSize: size + 1 } }),
        ...(compact ? { show: false } : {}),
      })),
    };
    for (const name of ["xAxis", "yAxis", "radiusAxis", "angleAxis"]) {
      if (authored[name]) result[name] = components(authored[name], (value) => axis(value, name));
    }
    if (authored.dataZoom) {
      result.dataZoom = components(authored.dataZoom, (value) => value.type === "inside" ? {
        ...value,
        // Keep pinch zoom while allowing one-finger page scrolling on phones.
        moveOnMouseMove: value.moveOnMouseMove ?? !small,
        preventDefaultMouseMove: value.preventDefaultMouseMove ?? !small,
      } : value);
    }
    if (authored.grid || authored.xAxis || authored.yAxis) {
      const multiple = Array.isArray(authored.grid) && authored.grid.length > 1;
      // ECharts 6 fits labels/names against the canvas itself. Forcing
      // containLabel also reserves label space inside an authored axis margin.
      result.grid = components(authored.grid ?? {}, (value) => ({
        left: 4, right: 4, top: authored.legend ? 40 : 16, bottom: 20, ...value,
        ...(compact && !multiple ? { top: 8, bottom: 18 } : {}),
      }));
    }
    if (compact) {
      if (authored.title) result.title = components(authored.title, (value) => ({ ...value, show: false }));
      if (authored.visualMap) result.visualMap = components(authored.visualMap, (value) => ({ ...value, show: false }));
      if (authored.series) result.series = components(authored.series, (value) => ({
        ...value, showSymbol: false, label: { ...value.label, show: false },
        ...(value.markLine ? { markLine: { ...value.markLine, label: { ...value.markLine.label, show: false } } } : {}),
      }));
    }
    return result;
  }

  function responsiveOption(option, compact) {
    const { media = [], baseOption, ...plain } = option;
    const zoomDefaults = (value) => {
      if (!value.dataZoom) return value;
      return { ...value, dataZoom: components(value.dataZoom, (zoom) => zoom.type === "inside" ? zoom : { show: false, ...zoom }) };
    };
    const authored = zoomDefaults(baseOption ?? plain);
    // ECharts media queries re-evaluate on resize, including phone rotation.
    // Explicit YAML attributes and authored media rules take precedence.
    const base = { backgroundColor: "transparent", animation: false, ...authored, ...viewDefaults(authored, false, compact) };
    const rules = [
      { query: { minWidth: 481 }, option: viewDefaults(authored, false, compact) },
      { query: { maxWidth: 480 }, option: viewDefaults(authored, true, compact) },
      ...media.map((rule) => ({ ...rule, option: zoomDefaults(rule.option) })),
    ];
    return { baseOption: base, media: rules };
  }

  window.facetraRender = (payload, dark, id, title, compact = false) => {
    renderID = id;
    try {
      const option = compile(payload);
      const language = (payload.locale ?? "en").toLowerCase().startsWith("zh") ? "ZH" : "EN";
      const previous = chart && locale !== payload.locale ? chart.getOption() : null;
      if (!chart || theme !== dark || (locale ?? "en").toLowerCase().startsWith("zh") !== (language === "ZH")) {
        chart?.dispose();
        chart = echarts.init(container, dark ? "dark" : null, { renderer: "svg", locale: language });
        theme = dark;
      }
      locale = payload.locale;
      container.setAttribute("aria-label", title);
      // Rendering can precede ResizeObserver after a layout change. Update
      // geometry first so endpoint label alignment uses the current width.
      chart.resize();
      chart.setOption(responsiveOption(option, compact), { notMerge: true });
      if (previous) {
        const current = chart.getOption();
        const names = new Map();
        for (const [index, series] of (previous.series ?? []).entries()) {
          const next = current.series?.find((item) => series.id && item.id === series.id) ?? current.series?.[index];
          if (next) names.set(series.name, next.name);
        }
        // Restore interaction state against the translated series names.
        // Keep zoom ranges independent of the new locale's axis formatting.
        const dataZoom = (current.dataZoom ?? []).map((zoom, index) => {
          const old = previous.dataZoom?.find((item) => zoom.id && item.id === zoom.id) ?? previous.dataZoom?.[index];
          return old ? { id: zoom.id, start: old.start, end: old.end } : { id: zoom.id };
        });
        const legend = (current.legend ?? []).map((item, index) => {
          const old = previous.legend?.[index];
          const selected = Object.fromEntries(Object.entries(old?.selected ?? {}).map(([name, value]) => [names.get(name) ?? name, value]));
          return { id: item.id, selected };
        });
        chart.setOption({ dataZoom, legend });
      }
      report("rendered");
    } catch (error) {
      chart?.clear();
      report("error", error instanceof Error ? error.message : String(error));
    }
  };
  window.addEventListener("error", (event) => report("error", event.message));
  window.addEventListener("unhandledrejection", (event) => report("error", String(event.reason)));
  new ResizeObserver(() => chart?.resize()).observe(container);
  window.addEventListener("pagehide", () => chart?.dispose());
  report("ready");
})();
