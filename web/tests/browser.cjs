// Run with Playwright available on NODE_PATH and an HTTP server at NOTIST_TEST_URL.
const assert = require('node:assert/strict');
const { chromium } = require('playwright');
(async () => {
  const browser = await chromium.launch({ headless: true, ...(process.env.NOTIST_CHROMIUM ? {executablePath: process.env.NOTIST_CHROMIUM} : {}) });
  try {
    const base = process.env.NOTIST_TEST_URL ?? 'http://127.0.0.1:8000';
    const page = await browser.newPage();
    page.setDefaultTimeout(90000);
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(`${base}${process.env.NOTIST_COMPONENTS_DEMO ?? '/target/components-demo/'}`);
    await page.waitForFunction(() => document.querySelector('mermaid-diagram')?.shadowRoot?.querySelector('svg'));
    await page.waitForFunction(() => document.querySelector('katex-math')?.shadowRoot?.querySelector('.katex'));
    await page.waitForFunction(() => [...document.querySelectorAll('typst-math')].length === 2 && [...document.querySelectorAll('typst-math')].every(node => node.shadowRoot?.querySelector('svg')));
    assert.equal(await page.locator('katex-math math').count(), 1);
    assert.equal(await page.evaluate(() => document.querySelectorAll('link[data-notist-katex]').length), 1);
    const formula = page.locator('katex-math').first();
    await formula.evaluate(node => node.setAttribute('notist-text', String.raw`\frac{1}{2}`));
    await page.waitForFunction(() => document.querySelector('katex-math').shadowRoot.querySelector('annotation')?.textContent === String.raw`\frac{1}{2}`);
    await formula.evaluate(node => node.setAttribute('notist-block', 'true'));
    await page.waitForFunction(() => document.querySelector('katex-math').shadowRoot.querySelector('.katex-display'));
    assert.equal(await formula.locator('math').getAttribute('display'), 'block');
    assert.equal(await formula.evaluate(node => getComputedStyle(node).display), 'block');
    await formula.evaluate(node => node.setAttribute('notist-block', 'false'));
    await page.waitForFunction(() => !document.querySelector('katex-math').shadowRoot.querySelector('.katex-display'));
    assert.equal(await formula.evaluate(node => getComputedStyle(node).display), 'inline');
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
      const panel = document.querySelector('notist-doc-panel');
      const root = panel.shadowRoot;
      panel.remove(); document.body.append(panel);
      return panel.shadowRoot === root;
    });
    assert.equal(reconnect, true);
    await page.waitForFunction(() => document.querySelector('mermaid-diagram')?.shadowRoot?.querySelector('svg'));
    assert.equal(await page.locator('notist-doc-badge').getAttribute('notist-count'), '9223372036854775807');
    await page.goto(`${base}/web/`);
    await page.waitForSelector('#core .kind');
    assert.equal(await page.locator('#config-url').inputValue(), '../docs/Notist.toml');
    assert.equal(await page.locator('#path').inputValue(), 'components/README.not');
    const loaded = page.waitForResponse(response => response.url() === `${base}/docs/components/README.not`);
    await page.click('#load-project');
    await loaded;
    await page.waitForFunction(() => document.querySelector('#core').textContent.includes('notist-doc::panel'));
    const frame = page.frames().find(frame => frame.url() === 'about:srcdoc');
    await frame.waitForFunction(() => document.querySelector('mermaid-diagram')?.shadowRoot?.querySelector('svg'));
    assert.equal(await frame.locator('notist-doc-panel').count(), 2);
    await frame.waitForFunction(() => document.querySelector('katex-math')?.shadowRoot?.querySelector('.katex'));
    await frame.waitForFunction(() => [...document.querySelectorAll('typst-math')].length === 2 && [...document.querySelectorAll('typst-math')].every(node => node.shadowRoot?.querySelector('svg')));
    assert.match(await page.locator('#core').textContent(), /Math/);
    // Keep Markdown math on the same configured transform and component path.
    await page.fill('#path', 'math.md');
    await page.locator('#path').dispatchEvent('change');
    await page.fill('#src', '- $x^2$\n\n$ y^2 $');
    await page.waitForFunction(() => document.querySelector('#core').textContent.includes('Math') && !document.querySelector('#core').textContent.includes('notist-doc::panel'));
    await page.waitForFunction(() => {
      const doc = document.querySelector('#preview').contentDocument;
      return doc && doc.querySelectorAll('typst-math').length === 2 && [...doc.querySelectorAll('typst-math')].every(node => node.shadowRoot?.querySelector('svg'));
    });
    const block = page.frameLocator('#preview').locator('typst-math[notist-block="true"]');
    assert.equal(await block.count(), 1);
    assert.equal(await block.evaluate(node => getComputedStyle(node).display), 'block');
    assert.equal(await block.evaluate(node => node.closest('p')), null);
    // Native tables let complete math payloads own pipes and blank lines.
    await page.fill('#path', 'math.not');
    await page.locator('#path').dispatchEvent('change');
    const payload = String.raw`f(x) &= x^2 + 2x + 1 \

     &= (x + 1)^2`;
    await page.fill('#src', `| package | content |\n| --- | --- |\n| typst | Typst 数学呈现 $\n${payload}\n$ |\n| absolute | $|x|$ |\n| next | row |`);
    await page.waitForFunction(payload => {
      const doc = document.querySelector('#preview').contentDocument;
      const nodes = [...(doc?.querySelectorAll('typst-math') ?? [])];
      return nodes.length === 2 && nodes[0].getAttribute('notist-text') === payload
        && nodes.every(node => node.shadowRoot?.querySelector('svg') && !node.shadowRoot.querySelector('.error').textContent);
    }, payload);
    const native = page.frameLocator('#preview');
    assert.equal(await native.locator('table tr').count(), 4);
    assert.equal(await native.locator('table tr').nth(1).locator('td').count(), 2);
    assert.equal(await native.locator('table tr').nth(1).locator('td').nth(1).locator('typst-math[notist-block="true"]').count(), 1);
    assert.equal(await native.locator('typst-math').nth(1).getAttribute('notist-text'), '|x|');
    assert.equal(await native.locator('table tr').last().textContent(), 'nextrow');
    assert.equal(await page.locator('#diags').textContent(), '✓ 无诊断');
    // Content paths respect the prepared Vault root; package dependency files
    // outside that root remain usable for declarations and components.
    await page.fill('#src', '[outside](../outside.not) ![asset](../outside.svg)');
    await page.waitForFunction(() => (document.querySelector('#diags').textContent.match(/outside Vault root/g) ?? []).length === 2);
    await page.fill('#src', '[inside](sub/../inside.not) ![remote](https://example.test/image.svg)');
    await page.waitForFunction(() => document.querySelector('#diags').textContent === '✓ 无诊断');
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
    await page.route('**/late/Notist.toml', route => route.fulfill({ body: '[package]\nname = "late"' }));
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
