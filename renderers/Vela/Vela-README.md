# Bundled LuxAlgo Vela

`vela.global.min.js` is the unmodified **@luxalgo/vela 0.8.4** browser build.
It runs locally in an ephemeral WKWebView. No provider or scripting engine is
registered; all bars come from Facetra's SQL result. The host uses native
indicators, keeps the upstream attribution visible, and blocks network access
with its document CSP. No eval permission is needed.

- Source: https://registry.npmjs.org/@luxalgo/vela/-/vela-0.8.4.tgz (`package/dist/vela.global.min.js`)
- Project: https://github.com/LuxAlgo/Vela
- License: [Vela-LICENSE.txt](Vela-LICENSE.txt), Apache-2.0
- Attribution notice: [Vela-NOTICE.txt](Vela-NOTICE.txt)
- Bundled dependency notices: [Vela-THIRD-PARTY-LICENSES.txt](Vela-THIRD-PARTY-LICENSES.txt) (Zag.js, Floating UI, proxy-compare)
- SHA-256: `4071c32b8cb65e5b75eefb5a6973babc1a90b3f875e44b7d5dad32be181a606f`

To update, download the exact npm release archive, extract the browser bundle,
LICENSE and NOTICE without modification, update the version and checksum here,
then run the Vela mapper and real WebKit integration tests. The full browser
build contains Vela's bundled UI dependencies; retain their distribution notices
when updating. PineTS and the separately licensed Pine addon are not included.
