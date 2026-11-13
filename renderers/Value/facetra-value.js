(() => {
  const root = document.getElementById("chart");
  const value = root.querySelector(".value");
  const send = (body) => window.webkit?.messageHandlers.chartStatus.postMessage(body);
  window.facetraRender = (payload, dark, id, title) => {
    try {
      root.setAttribute("role", "img");
      root.setAttribute("aria-label", title || "");
      value.textContent = payload.value == null ? "—" : String(payload.value);
      send({ status: "rendered", id });
    } catch (error) {
      send({ status: "error", id, message: String(error.message || error) });
    }
  };
  send({ status: "ready", id: 0 });
})();
