// Run with Playwright available on NODE_PATH and an HTTP server at NOTIST_TEST_URL.
const assert = require('node:assert/strict');
const { chromium } = require('playwright');
(async () => {
  const browser = await chromium.launch({ headless: true, ...(process.env.NOTIST_CHROMIUM ? {executablePath: process.env.NOTIST_CHROMIUM} : {}) });
  try {
    const base = process.env.NOTIST_TEST_URL ?? 'http://127.0.0.1:8000';
    const page = await browser.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(`${base}/target/packages-demo/`);
    await page.waitForFunction(() => document.querySelector('mermaid-diagram')?.shadowRoot?.querySelector('svg'));
    const reconnect = await page.evaluate(() => {
      const panel = document.querySelector('widgets-panel');
      const root = panel.shadowRoot;
      panel.remove(); document.body.append(panel);
      return panel.shadowRoot === root;
    });
    assert.equal(reconnect, true);
    await page.waitForFunction(() => document.querySelector('mermaid-diagram')?.shadowRoot?.querySelector('svg'));
    assert.equal(await page.locator('widgets-badge').getAttribute('notist-count'), '9223372036854775807');
    await page.goto(`${base}/web/`);
    await page.waitForSelector('#core .kind');
    assert.equal(await page.locator('#config-url').inputValue(), '../docs/Notist.toml');
    assert.equal(await page.locator('#path').inputValue(), 'packages/README.not');
    const loaded = page.waitForResponse(response => response.url() === `${base}/docs/packages/README.not`);
    await page.click('#load-project');
    await loaded;
    await page.waitForFunction(() => document.querySelector('#core').textContent.includes('widgets::panel'));
    const frame = page.frames().find(frame => frame.url() === 'about:srcdoc');
    await frame.waitForFunction(() => document.querySelector('mermaid-diagram')?.shadowRoot?.querySelector('svg'));
    assert.equal(await frame.locator('widgets-panel').count(), 2);
    await page.fill('#path', 'lib.notc');
    await page.locator('#path').dispatchEvent('change');
    await page.fill('#src', 'fn bad(value: Int = false) -> Content; fn good() -> Content;');
    await page.waitForFunction(() => document.querySelector('#tree').textContent.includes('Module') && document.querySelector('#diags').textContent.includes('default'));
    // A slow previous environment must not replace a later explicit reset.
    let releaseConfig;
    let configRequested;
    const requested = new Promise(resolve => { configRequested = resolve; });
    const gate = new Promise(resolve => { releaseConfig = resolve; });
    await page.route('**/slow.toml', async route => {
      configRequested();
      await gate;
      await route.fulfill({ body: '[dependencies]\nlate = {path = "./late"}' });
    });
    await page.route('**/late/lib.notc', route => route.fulfill({ body: 'fn flag() -> Content;' }));
    await page.route('**/late/components/**', route => route.fulfill({ status: 404, body: '' }));
    await page.fill('#path', 'scratch.not');
    await page.locator('#path').dispatchEvent('change');
    await page.fill('#src', '#late::flag()');
    await page.fill('#config-url', '../slow.toml');
    await page.click('#load-project');
    await requested;
    await page.fill('#config-url', '');
    await page.click('#load-project');
    const lastCheck = page.waitForResponse(response => response.url().endsWith('/late/components/flag/index.js'));
    releaseConfig();
    await lastCheck;
    // Allow the fulfilled response and its continuation to reach the page.
    await page.waitForTimeout(50);
    assert.match(await page.locator('#diags').textContent(), /unknown constructor/);
    assert.deepEqual(errors, []);
    console.log('Static HTML, reconnection, project preview and module diagnostics passed.');
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
