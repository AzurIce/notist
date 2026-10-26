// Serve the repository root with the built WASM and Playwright on NODE_PATH.
const assert = require('node:assert/strict');
const { chromium } = require('playwright');

(async () => {
  const browser = await chromium.launch({ headless: true, ...(process.env.NOTIST_CHROMIUM ? { executablePath: process.env.NOTIST_CHROMIUM } : {}) });
  try {
    const base = process.env.NOTIST_TEST_URL ?? 'http://127.0.0.1:8000';
    const page = await browser.newPage();
    const errors = [], requests = [];
    page.on('pageerror', error => errors.push(error.message));
    page.on('request', request => requests.push(new URL(request.url()).pathname));
    let renderer;
    await page.route(`${base}/docs/diagram-switch.toml`, route => route.fulfill({ body: `
[[extensions]]
kind = "replace"
imports = { components = "components/lib.notc", renderer = "../modules/${renderer}/lib.notc" }
from = "components.diagram"
to = "renderer.diagram"
` }));
    await page.route(`${base}/docs/diagram-demo.not`, route => route.fulfill({ body:
      '#import "components/lib.notc" as ui;\n#ui.diagram("flowchart LR; Source --> Item")'
    }));
    await page.goto(`${base}/web/`);
    await page.waitForSelector('#core .kind');
    await page.fill('#root-url', '../docs/');
    await page.fill('#config-url', 'diagram-switch.toml');
    await page.fill('#path', 'diagram-demo.not');
    for (const engine of ['pele', 'mermaid']) {
      renderer = engine;
      await page.click('#load-project');
      await page.waitForFunction(engine => {
        const document = window.document.querySelector('#preview').contentDocument;
        const diagram = document?.querySelector(`[notist-function$="/${engine}/lib.notc.diagram"]`);
        return diagram?.renderedSvg?.textContent.includes('Source');
      }, engine);
      assert.equal(await page.locator('#diags').textContent(), '✓ 无诊断');
      assert.match(await page.locator('#core').textContent(), /\/docs\/components\/lib\.notc\.diagram/);
      assert.equal(await page.frameLocator('#preview').locator('[notist-function$=".diagram"]').getAttribute('notist-theme'), 'auto');
      assert.ok(requests.includes(`/modules/${engine}/diagram/index.js`), `loaded ${engine} target`);
    }
    assert.ok(!requests.some(path => path.startsWith('/docs/components/diagram')), 'the semantic declaration needs no Web Component');
    assert.deepEqual(errors, []);
    console.log('Project diagram preserves semantic identity and switches Pele/Mermaid through config in the browser host.');
  } finally {
    await browser.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
