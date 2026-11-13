import { readFile } from "node:fs/promises";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";

const require = createRequire(import.meta.url);
const playwright = require("playwright");
const jobs = JSON.parse(readFileSync(0, "utf8"));

const bridge = `
window.__facetraStatus = { status: "loading" };
window.webkit = {
  messageHandlers: {
    chartStatus: {
      postMessage(message) {
        window.__facetraStatus = message;
      }
    }
  }
};
`;

function engineFiles(engine) {
  switch (engine) {
    case "ECharts":
      return { host: "echarts-host.html", vendors: ["echarts.min.js"], adapter: "facetra-echarts.js" };
    case "Vela":
      return { host: "vela-host.html", vendors: ["vela.global.min.js"], adapter: "facetra-vela.js" };
    case "VegaLite":
      return { host: "vegalite-host.html", vendors: ["vega.min.js", "vega-lite.min.js"], adapter: "facetra-vegalite.js" };
    case "HTML":
      return { host: "html-host.html", vendors: ["mustache.min.js"], adapter: "facetra-html.js" };
    case "Value":
      return { host: "value-host.html", vendors: [], adapter: "facetra-value.js" };
    default:
      throw new Error(`Unsupported renderer engine: ${engine}`);
  }
}

async function htmlFor(job) {
  const directory = path.join(job.rendererDir, job.engine);
  const files = engineFiles(job.engine);
  let html = await readFile(path.join(directory, files.host), "utf8");
  html = html.replace(/<meta[^>]+Content-Security-Policy[^>]*>/gi, "");
  return { html, directory, files };
}

async function addScripts(page, directory, files) {
  await page.addScriptTag({ content: bridge });
  for (const vendor of files.vendors) {
    await page.addScriptTag({ content: await readFile(path.join(directory, vendor), "utf8") });
  }
  await page.addScriptTag({
    content: await readFile(path.join(directory, files.adapter), "utf8"),
  });
}

const browser = await playwright.chromium.launch({ headless: true });
const results = [];
try {
  for (const job of jobs) {
    let context;
    try {
      context = await browser.newContext({
        viewport: { width: job.width, height: job.height },
        deviceScaleFactor: job.scale,
        colorScheme: "light",
      });
      const page = await context.newPage();
      const { html, directory, files } = await htmlFor(job);
      await page.setContent(html, { waitUntil: "load" });
      await addScripts(page, directory, files);
      await page.evaluate(
        ({ payload, title }) => window.facetraRender(payload, false, 1, title, false),
        { payload: job.payload, title: job.title },
      );
      await page.waitForFunction(
        () => ["rendered", "error"].includes(window.__facetraStatus?.status),
        null,
        { timeout: 30_000 },
      );
      const status = await page.evaluate(() => window.__facetraStatus);
      if (status.status === "error") {
        throw new Error(status.message || "The chart renderer rejected the configuration.");
      }
      await page.evaluate(() => {
        document.documentElement.style.background = "#ffffff";
        document.body.style.background = "#ffffff";
      });
      await page.screenshot({ path: job.out, type: "png", animations: "disabled" });
      results.push({ ok: true });
    } catch (error) {
      results.push({ ok: false, error: String(error?.message || error) });
    } finally {
      await context?.close();
    }
  }
} finally {
  await browser.close();
}
process.stdout.write(JSON.stringify(results));
