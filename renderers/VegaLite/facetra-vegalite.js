(() => {
  const root = document.getElementById('chart');
  const send = body => window.webkit?.messageHandlers.chartStatus.postMessage(body);
  let view, ticket = 0, last, resize;
  window.facetraRender = async (payload, dark, id, title, compact) => {
    const current = ++ticket;
    last = [payload, dark, id, title, compact];
    try {
      if (view) { view.finalize(); view = null; }
      root.replaceChildren();
      root.setAttribute('aria-label', title || '');
      const spec = structuredClone(payload.option);
      const foreground = dark ? '#e5e5e7' : '#262628', muted = dark ? '#a1a1a6' : '#66666b';
      document.documentElement.style.colorScheme = dark ? 'dark' : 'light';
      document.body.style.color = foreground;
      const defaults = {
        background: 'transparent',
        axis: {labelColor: muted, titleColor: foreground, gridColor: dark ? '#363639' : '#e5e5e7', labelFontSize: compact ? 10 : 12},
        legend: {labelColor: muted, titleColor: foreground, labelFontSize: compact ? 10 : 12},
        title: {color: foreground}, view: {stroke: 'transparent'}
      };
      const authored = spec.config || {};
      spec.config = {...defaults, ...authored};
      for (const key of ['axis', 'legend', 'title', 'view']) spec.config[key] = {...defaults[key], ...authored[key]};
      if (spec.background === undefined) spec.background = 'transparent';
      // Let Vega-Lite fit axes and legends within the full card width instead of
      // reserving a fixed horizontal margin. Authored/composed sizes stay intact.
      if ((spec.mark || spec.layer) && !spec.encoding?.row && !spec.encoding?.column) {
        if (spec.width === undefined) spec.width = 'container';
        if (spec.height === undefined || spec.height === 'container') spec.height = Math.max(60, root.clientHeight - 80);
      }
      const runtime = vega.parse(vegaLite.compile(spec).spec);
      const next = new vega.View(runtime, {renderer: 'svg', loader: {
        load: () => Promise.reject(new Error('External resources are unavailable in offline charts.')),
        sanitize: () => Promise.reject(new Error('External resources are unavailable in offline charts.'))
      }}).initialize(root).hover();
      view = next;
      await next.runAsync();
      if (current === ticket) send({status: 'rendered', id});
    } catch (error) {
      if (current === ticket) send({status: 'error', id, message: String(error.message || error)});
    }
  };
  new ResizeObserver(() => {
    clearTimeout(resize);
    if (last) resize = setTimeout(() => window.facetraRender(...last), 80);
  }).observe(root);
  window.facetraDispose = () => { ++ticket; clearTimeout(resize); last = null; view?.finalize(); view = null; root.replaceChildren(); };
  send({status: 'ready', id: 0});
})();
