// Run with Playwright on NODE_PATH and the built WASM served by NOTIST_TEST_URL.
const assert = require('node:assert/strict');
const { chromium } = require('playwright');
(async () => {
  const browser = await chromium.launch({ headless: true, ...(process.env.NOTIST_CHROMIUM ? { executablePath: process.env.NOTIST_CHROMIUM } : {}) });
  try {
    const page = await browser.newPage();
    await page.goto(`${process.env.NOTIST_TEST_URL ?? 'http://127.0.0.1:8000'}/web/`);
    const result = await page.evaluate(async () => {
      const wasm = await import('/web/pkg/notist.js');
      await wasm.default();
      const bytes = text => Array.from(new TextEncoder().encode(text));
      const inputs = { root: '/code', config: '/code/Notist.toml', files: {
        '/code/Notist.toml': bytes(''),
        '/code/lib.notc': bytes('fn bad() -> InlineContent { compile_error("broken child") } fn good() -> InlineContent { [good child] } fn outer() -> InlineContent { [#bad() #good()] }'),
      }, module_urls: {} };
      return JSON.parse(wasm.analyze_prepared('doc.not', '#import "lib.notc" as demo;\n#demo.outer()', JSON.stringify(inputs)));
    });
    const nodes = root => [root, ...(root.children ?? []).flatMap(nodes)];
    assert.equal(nodes(result.core).filter(node => node.function === '/code/lib.notc.bad').length, 0);
    assert.equal(nodes(result.core).find(node => node.function === '/code/lib.notc.outer').pending.kind, 'call');
    const evaluated = nodes(result.evaluated);
    assert(evaluated.every(node => node.pending === null));
    assert.equal(evaluated.find(node => node.function === '/code/lib.notc.outer').evaluation, 'expanded');
    assert.match(evaluated.find(node => node.function === '/code/lib.notc.bad').evaluation.error, /broken child/);
    assert.equal(evaluated.find(node => node.function === '/code/lib.notc.good').evaluation, 'expanded');
    assert.match(result.html, /broken child/);
    assert.match(result.html, /good child/);
    assert.equal(result.used_components.length, 0);
    assert(result.source_map.some(mapping => mapping.source === '/code/lib.notc' && mapping.expansion.some(location => location.source === '/code/doc.not')));
    const module = await page.evaluate(async () => {
      const wasm = await import('/web/pkg/notist.js');
      const bytes = text => Array.from(new TextEncoder().encode(text));
      const source = 'fn value() -> String { "ok" } fn view() -> InlineContent { [#text(text: self.value())] }';
      const inputs = { root: '/code', files: {
        '/code/Notist.toml': bytes(''),
        '/code/lib.notc': bytes('fn stale() -> InlineContent { compile_error("stale") }'),
      } };
      return JSON.parse(wasm.analyze_prepared('/code/lib.notc', source, JSON.stringify(inputs)));
    });
    assert.equal(module.functions[1].function, '/code/lib.notc.view');
    assert.deepEqual(module.diagnostics, []);

    const requests = [];
    await page.route('**/import-edit/**', route => {
      const file = new URL(route.request().url()).pathname.split('/').pop();
      requests.push(file);
      const files = {
        'doc.not': '#import "first.notc" as selected;\n#selected.view()',
        'first.notc': 'fn view() -> InlineContent { [first imported value] }',
        'second.notc': 'fn view() -> InlineContent { [second imported value] }',
      };
      return route.fulfill({ status: file in files ? 200 : 404, body: files[file] ?? '' });
    });
    await page.waitForSelector('#core .kind');
    await page.fill('#root-url', '../import-edit/');
    await page.fill('#config-url', '');
    await page.fill('#path', 'doc.not');
    await page.click('#load-project');
    await page.waitForFunction(() => document.querySelector('#preview').contentDocument?.body?.textContent.includes('first imported value'));
    await page.fill('#src', '#import "second.notc" as selected;\n#selected.view()');
    await page.waitForFunction(() => document.querySelector('#preview').contentDocument?.body?.textContent.includes('second imported value'));
    assert.equal(await page.locator('#diags').textContent(), '✓ 无诊断');
    assert.equal(requests.filter(file => file === 'first.notc').length, 1);
    assert.equal(requests.filter(file => file === 'second.notc').length, 1);
    assert.equal(requests.includes('lib.notc'), false);
    await page.route('**/import-edit/native.notc', route => route.fulfill({ body: 'fn view() -> InlineContent;' }));
    await page.route('**/import-edit/view.js', route => route.fulfill({ status: 503, body: '' }));
    await page.route('**/import-edit/view/index.js', route => route.fulfill({ status: 404, body: '' }));
    await page.fill('#src', '#import "native.notc" as changed;\n#changed.view()');
    await page.waitForFunction(() => document.querySelector('#diags').textContent.includes('503'));
    assert.match(await page.locator('#core').textContent(), /native.notc.view/);
    assert.doesNotMatch(await page.locator('#core').textContent(), /second.notc.view/);
    console.log('WASM Code checking, evaluation, error recovery, source expansion and edited source import graphs passed.');
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
