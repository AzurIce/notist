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
        '/code/Notist.toml': bytes('[package]\nname = "demo"'),
        '/code/lib.notc': bytes('fn bad() -> InlineContent { compile_error("broken child") } fn good() -> InlineContent { [good child] } fn outer() -> InlineContent { [#bad() #good()] }'),
      }, module_urls: {} };
      return JSON.parse(wasm.analyze_prepared('doc.not', '#demo::outer()', JSON.stringify(inputs)));
    });
    const nodes = root => [root, ...(root.children ?? []).flatMap(nodes)];
    assert.equal(nodes(result.core).filter(node => node.function === 'demo::bad').length, 0);
    assert.equal(nodes(result.core).find(node => node.function === 'demo::outer').pending.kind, 'call');
    const evaluated = nodes(result.evaluated);
    assert(evaluated.every(node => node.pending === null));
    assert.equal(evaluated.find(node => node.function === 'demo::outer').evaluation, 'expanded');
    assert.match(evaluated.find(node => node.function === 'demo::bad').evaluation.error, /broken child/);
    assert.equal(evaluated.find(node => node.function === 'demo::good').evaluation, 'expanded');
    assert.match(result.html, /broken child/);
    assert.match(result.html, /good child/);
    assert.equal(result.used_components.length, 0);
    assert(result.source_map.some(mapping => mapping.source === '/code/lib.notc' && mapping.expansion.some(location => location.source === 'doc.not')));
    const module = await page.evaluate(async () => {
      const wasm = await import('/web/pkg/notist.js');
      const bytes = text => Array.from(new TextEncoder().encode(text));
      const source = 'fn value() -> String { "ok" } fn view() -> InlineContent { [#text(text: demo::value())] }';
      const inputs = { root: '/code', files: {
        '/code/Notist.toml': bytes('[package]\nname = "demo"'),
        '/code/lib.notc': bytes('fn stale() -> InlineContent { compile_error("stale") }'),
      } };
      return JSON.parse(wasm.analyze_prepared('/code/lib.notc', source, JSON.stringify(inputs)));
    });
    assert.equal(module.functions[1].function, 'demo::view');
    assert.deepEqual(module.diagnostics, []);
    console.log('WASM Code checking, opaque semantic trees, nested error recovery, evaluation and source expansion passed.');
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
