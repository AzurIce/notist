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
    await page.waitForFunction(() => [...document.querySelectorAll('katex-math')].length === 3 && [...document.querySelectorAll('katex-math')].every(node => node.shadowRoot?.querySelector('.katex')));
    assert.equal(await page.locator('katex-math math').count(), 3);
    assert.equal(await page.evaluate(() => document.querySelectorAll('link[data-notist-katex]').length), 1);
    const formula = page.locator('katex-math').first();
    await formula.evaluate(node => node.setAttribute('notist-text', String.raw`\frac{1}{2}`));
    await page.waitForFunction(() => document.querySelector('katex-math').shadowRoot.querySelector('annotation')?.textContent === String.raw`\frac{1}{2}`);
    const reconnected = await formula.evaluate(node => {
      const root = node.shadowRoot;
      const parent = node.parentNode;
      const next = node.nextSibling;
      node.remove();
      parent.insertBefore(node, next);
      return node.shadowRoot === root;
    });
    assert.equal(reconnected, true);
    await formula.evaluate(node => node.setAttribute('notist-text', String.raw`\unknownNotistCommand`));
    await page.waitForFunction(() => document.querySelector('katex-math').shadowRoot.querySelector('.error')?.textContent.includes('Math error'));
    assert.equal(await formula.locator('.formula').textContent(), String.raw`\unknownNotistCommand`);
    await formula.evaluate(node => node.setAttribute('notist-text', 'x^2'));
    await page.waitForFunction(() => document.querySelector('katex-math').shadowRoot.querySelector('annotation')?.textContent === 'x^2');
    assert.equal(await formula.locator('.error').textContent(), '');
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
    assert.equal(await page.locator('#config-url').inputValue(), '../packages/widgets/Notist.toml');
    assert.equal(await page.locator('#path').inputValue(), 'README.not');
    const loaded = page.waitForResponse(response => response.url() === `${base}/packages/widgets/README.not`);
    await page.click('#load-project');
    await loaded;
    await page.waitForFunction(() => document.querySelector('#core').textContent.includes('widgets::panel'));
    const frame = page.frames().find(frame => frame.url() === 'about:srcdoc');
    await frame.waitForFunction(() => document.querySelector('mermaid-diagram')?.shadowRoot?.querySelector('svg'));
    assert.equal(await frame.locator('widgets-panel').count(), 2);
    await frame.waitForFunction(() => [...document.querySelectorAll('katex-math')].length === 3 && [...document.querySelectorAll('katex-math')].every(node => node.shadowRoot?.querySelector('.katex')));
    assert.match(await page.locator('#core').textContent(), /Math/);
    // Keep Markdown math on the same configured transform and component path.
    await page.fill('#path', 'math.md');
    await page.locator('#path').dispatchEvent('change');
    await page.fill('#src', '- $x^2$\n\n$y^2$');
    await page.waitForFunction(() => document.querySelector('#core').textContent.includes('Math') && !document.querySelector('#core').textContent.includes('widgets::panel'));
    await page.waitForFunction(() => {
      const doc = document.querySelector('#preview').contentDocument;
      return doc && doc.querySelectorAll('katex-math').length === 2 && [...doc.querySelectorAll('katex-math')].every(node => node.shadowRoot?.querySelector('.katex'));
    });
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
