(() => {
  const root = document.getElementById('chart');
  const send = body => window.webkit?.messageHandlers.chartStatus.postMessage(body);
  let frame;
  window.facetraRender = (payload, dark, id, title) => {
    try {
      const option = payload.option;
      // Data substitutions are escaped by Mustache. No authored JavaScript is evaluated.
      const html = Mustache.render(option.html, option.data || {});
      frame?.remove();
      frame = document.createElement('iframe');
      frame.title = title || '';
      frame.setAttribute('sandbox', '');
      const child = frame;
      frame.srcdoc = `<!doctype html><html class="${dark ? 'dark' : ''}"><head><meta name="viewport" content="width=device-width, initial-scale=1">
        <meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; img-src data:; connect-src 'none'; form-action 'none'; base-uri 'none'">
        <style>html,body{margin:0;font:14px system-ui;color:var(--text);background:transparent}*{box-sizing:border-box}a{color:var(--accent)} :root{--text:#262628;--muted:#68686d;--card:#f4f4f5;--border:#dedee1;--accent:#007aff} :root.dark{color-scheme:dark;--text:#e5e5e7;--muted:#a1a1a6;--card:#29292c;--border:#414145;--accent:#5aa7ff}</style>
        <style>${(option.css || '').replace(/<\/style/gi, '<\\/style')}</style></head><body>${html}</body></html>`;
      frame.onload = () => { if (frame === child) send({status: 'rendered', id}); };
      root.append(frame);
    } catch (error) { send({status: 'error', id, message: String(error.message || error)}); }
  };
  window.facetraDispose = () => { frame?.remove(); frame = null; };
  send({status: 'ready', id: 0});
})();
