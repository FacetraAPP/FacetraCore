# Offline renderer assets

These files are shared by the Facetra app and `facetra-render`.

Install the browser dependency:

```sh
npm install --prefix renderers
npx --prefix renderers playwright install chromium
```

`capture.mjs` starts a headless Chromium process, loads the matching offline
host, injects the vendored renderer scripts after the document body exists,
waits for `window.__facetraStatus` to become `rendered`, and captures a PNG.

Do not replace the vendored JavaScript files without updating their upstream
license and notice files.

