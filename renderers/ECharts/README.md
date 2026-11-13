# Bundled Apache ECharts

`echarts.min.js` is the unmodified full Apache ECharts **6.1.0** distribution,
including all 23 core series and the matrix coordinate system.
It is loaded locally in an ephemeral WKWebView, without a CDN or remote assets.

- Source: https://raw.githubusercontent.com/apache/echarts/6.1.0/dist/echarts.min.js
- License: [ECharts-LICENSE.txt](ECharts-LICENSE.txt) (upstream LICENSE, including third-party notices)
- Notice: [ECharts-NOTICE.txt](ECharts-NOTICE.txt)

`facetra-echarts.js` and `echarts-host.html` are Facetra's generic WebKit host.
The host applies the YAML option, compiles only explicitly identified script
locations, responds to container resizing and reports render errors to Swift.
The document's CSP blocks network connections and external resource loading;
its native bridge accepts chart status messages only. Generic container media
queries reduce label density on phones, authored media rules take precedence,
and dataZoom sliders are hidden unless YAML explicitly sets `show: true`.

SHA-256 of the upstream distribution:
`b66b25aeb4df84e33199dc21694014d336d222cbd9deb0e5a7c14bd6aa0d0fd0`
