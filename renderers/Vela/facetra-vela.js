/* Offline Vela host. All market data comes from Facetra's typed SQL result. */
(() => {
  const container = document.getElementById("chart");
  let chart;
  let renderID = 0;
  let cancelRender;
  let fitView;
  let userAdjustedView = false;
  const report = (status, id, message) => window.webkit.messageHandlers.chartStatus.postMessage({ status, id, message });
  const message = (error) => error instanceof Error ? error.message : String(error);

  function validateInputs(handle, inputs) {
    for (const [key, value] of Object.entries(inputs)) {
      const schema = handle.inputs.find((input) => input.key === key);
      if (!schema) throw new Error(`${handle.nativeType}: unknown input ${key}.`);
      const numeric = ["int", "float", "price", "time"].includes(schema.type);
      const valid = numeric ? typeof value === "number" && Number.isFinite(value)
        && (schema.type !== "int" || Number.isInteger(value))
        && (schema.min == null || value >= schema.min) && (schema.max == null || value <= schema.max)
        : schema.type === "bool" ? typeof value === "boolean" : typeof value === "string";
      if (!valid || (schema.options && !schema.options.includes(value))) throw new Error(`${handle.nativeType}: invalid input ${key}.`);
    }
  }

  window.facetraRender = async (payload, dark, id, title, compact = false) => {
    renderID = id;
    cancelRender?.();
    const cancelled = new Promise((resolve) => { cancelRender = resolve; });
    chart?.destroy();
    chart = undefined;
    window.facetraVela = undefined;
    fitView = undefined;
    userAdjustedView = false;
    let current;
    try {
      container.setAttribute("aria-label", title);
      current = new Vela.Vela(container, {
        data: payload.bars,
        timeframe: payload.timeframe,
        theme: dark ? "dark" : "light",
        priceStyle: "candles",
        live: false,
        drawings: false,
        animations: false,
        volume: false,
      });
      chart = current;
      // Keep reset-to-latest compact too; offline cards need no forecast area.
      current.renderer.applyConfig({ margins: { right: 1 } });
      if (compact) current.renderer.set("indicatorTitles", false);
      current.on("indicator:error", ({ error }) => {
        if (id === renderID) report("error", id, message(error));
      });
      await Promise.race([current.ready(), cancelled]);
      if (id !== renderID) return;
      container.querySelectorAll('a[href^="https://velacharts.dev"]').forEach((link) => { link.target = "_self"; });
      const catalog = await Promise.race([current.availableNativeIndicators(), cancelled]);
      if (id !== renderID) return;
      const seen = new Set();
      const indicators = [...payload.indicators];
      if (payload.showVolume && !compact && !indicators.some((item) => item.type === "volume")) indicators.unshift({ type: "volume", inputs: {} });
      for (const indicator of indicators) {
        if (compact && indicator.type === "volume") continue;
        const info = catalog.find((entry) => entry.type === indicator.type);
        if (!info?.supported) throw new Error(`Unknown or unsupported Vela indicator: ${indicator.type}.`);
        if (!info.multiInstance && seen.has(info.type)) throw new Error(`Only one ${info.type} indicator is allowed.`);
        seen.add(info.type);
        const handle = current.addNativeIndicator(indicator.type, { inputs: indicator.inputs });
        validateInputs(handle, indicator.inputs);
        // Compact cards keep the price overlays; oscillator panes need the full view.
        await Promise.race([cancelled, new Promise((resolve, reject) => {
          handle.on("ready", resolve);
          handle.on("error", ({ error }) => reject(error));
        })]);
        if (id !== renderID) return;
        if (compact) {
          const model = current.inspect().indicators.find((item) => item.id === handle.id);
          if (model?.overlay === false) handle.remove();
        }
      }
      fitView = () => {
        if (chart !== current || !payload.bars.length) return;
        // The outer host can resize before Vela's own observer runs. Refresh
        // its geometry first so the range uses the new plot width immediately.
        current.resize();
        // A single observation has no inferred interval in Vela. Its native
        // initial fit centers the candle; a timestamp range would clip it.
        if (payload.bars.length === 1) return;
        const interval = Vela.timeframeToMs(payload.timeframe);
        const first = payload.bars[Math.max(0, payload.bars.length - 200)];
        const last = payload.bars[payload.bars.length - 1];
        // Half a bar at each edge keeps complete candles visible without a
        // disproportionate blank area after the final observation.
        current.setVisibleRange({ from: first.time - interval / 2, to: last.time + interval / 2 });
      };
      fitView();
      // Expose the public instance for diagnostics and WebKit integration checks.
      window.facetraVela = current;
      report("rendered", id);
    } catch (error) {
      if (id !== renderID) return;
      current?.destroy();
      chart = undefined;
      report("error", id, message(error));
    }
  };
  window.facetraDispose = () => {
    renderID += 1;
    cancelRender?.();
    fitView = undefined;
    chart?.destroy();
    chart = undefined;
    window.facetraVela = undefined;
  };
  // Keep the initial history fitted through native layout changes and rotation.
  // Once the user pans/zooms, retain their viewport instead of resetting it.
  for (const type of ["pointerdown", "wheel", "keydown"]) container.addEventListener(type, () => { userAdjustedView = true; }, { passive: true });
  const resizeObserver = new ResizeObserver(() => {
    if (!userAdjustedView) fitView?.();
  });
  resizeObserver.observe(container);
  window.addEventListener("pagehide", window.facetraDispose);
  window.addEventListener("pagehide", () => resizeObserver.disconnect());
  window.addEventListener("error", (event) => report("error", renderID, event.message));
  window.addEventListener("unhandledrejection", (event) => report("error", renderID, message(event.reason)));
  report("ready", 0);
})();
