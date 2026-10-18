// Serve the repository and make Playwright available on NODE_PATH.
// NOTIST_TEST_URL and NOTIST_CHROMIUM select the HTTP server and browser.
const assert = require('node:assert/strict');
const { chromium } = require('playwright');
const { mkdir } = require('node:fs/promises');
const { join } = require('node:path');

(async () => {
  const browser = await chromium.launch({
    headless: true,
    ...(process.env.NOTIST_CHROMIUM ? { executablePath: process.env.NOTIST_CHROMIUM } : {}),
  });
  try {
    const page = await browser.newPage({ viewport: { width: 1000, height: 1600 } });
    const base = process.env.NOTIST_TEST_URL ?? 'http://127.0.0.1:8000';
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(`${base}/packages/mermaid/`);
    await page.setContent(`<!doctype html><style>
      body { margin: 24px; font: 16px sans-serif; }
      /* Mermaid's temporary light-DOM labels must not inherit these rules. */
      p { font-size: 8px !important; white-space: nowrap !important; }
      .label { font-size: 9px !important; }
      mermaid-diagram { display: block; max-width: 680px; }
    </style><main></main>`);
    await page.evaluate(async base => {
      const { default: Diagram } = await import(`${base}/packages/mermaid/components/diagram/index.js`);
      customElements.define('mermaid-diagram', Diagram);
    }, base);
    const source = `flowchart TB
      subgraph parsing["解析 · 字体与布局"]
        A["局部 Resolve → Shape → Materialize"]
        B["递归展开返回内容，以 Node 承接调用位置"]
        A -->|"返回内容 · Markup 模板与插值"| B
      end
      B --> C["ImplementationEnvironmentWithAnUnbrokenLongIdentifier"]
    `;
    await page.evaluate(source => {
      const diagram = document.createElement('mermaid-diagram');
      diagram.setAttribute('notist-source', source);
      diagram.setAttribute('notist-theme', 'neutral');
      document.querySelector('main').append(diagram);
    }, source);
    const diagram = page.locator('mermaid-diagram');
    const waitForSvg = (previousId = null) => page.waitForFunction(id => {
      const svg = document.querySelector('mermaid-diagram')?.renderedSvg;
      return svg && svg.id !== id;
    }, previousId, { timeout: 90000 });
    await waitForSvg();
    const assertLabels = async (mode = 'html') => {
      const htmlCount = await diagram.locator('foreignObject').count();
      assert.ok(mode === 'html' ? htmlCount > 0 : htmlCount === 0, `${mode} label mode`);
      const clipped = await diagram.evaluate(node => {
        const tolerance = 2;
        return [...node.querySelectorAll('foreignObject')].flatMap(fo => {
          if (!fo.textContent.trim()) return [];
          const bounds = fo.getBoundingClientRect();
          const walker = document.createTreeWalker(fo, NodeFilter.SHOW_TEXT);
          for (let text = walker.nextNode(); text; text = walker.nextNode()) {
            if (!text.textContent.trim()) continue;
            const range = document.createRange();
            range.selectNodeContents(text);
            if ([...range.getClientRects()].some(rect =>
              rect.left < bounds.left - tolerance || rect.top < bounds.top - tolerance
              || rect.right > bounds.right + tolerance || rect.bottom > bounds.bottom + tolerance
            )) return [fo.textContent];
          }
          return [];
        });
      });
      assert.deepEqual(clipped, [], 'HTML text must fit inside its measured viewport');
      const measurements = await diagram.evaluate(node =>
        [...node.querySelectorAll('g.node')].map(group => {
          const text = group.querySelector('foreignObject, text');
          const shape = group.querySelector('rect, polygon, ellipse, circle, path');
          const label = text?.getBoundingClientRect();
          const box = shape?.getBoundingClientRect();
          return {
            text: text?.textContent,
            fits: label && box && label.width > 0 && label.height > 0
              && label.x >= box.x - 2 && label.y >= box.y - 2
              && label.x + label.width <= box.x + box.width + 2
              && label.y + label.height <= box.y + box.height + 2,
          };
        })
      );
      assert.ok(measurements.length > 0);
      assert.ok(measurements.every(label => label.fits), JSON.stringify(measurements));
    };
    const assertTextVisible = async expected => {
      assert.equal(await diagram.evaluate((node, expected) =>
        [...node.querySelectorAll('svg text, .nodeLabel, .edgeLabel')].some(label => {
          const matches = label.textContent.replace(/\s+/g, '') === expected.replace(/\s+/g, '');
          const bounds = label.getBoundingClientRect();
          return matches && bounds.width > 0 && bounds.height > 0;
        }), expected
      ), true, `complete visible text: ${expected}`);
    };
    await assertLabels();
    await assertTextVisible('解析 · 字体与布局');
    await assertTextVisible('返回内容 · Markup 模板与插值');
    await page.addStyleTag({ content: `
      .diagram > svg .nodeLabel p { font-size: 40px !important; }
      .diagram > div > svg .nodeLabel p { font-size: 12px !important; }
    ` });
    const beforeAncestry = await diagram.locator('svg').getAttribute('id');
    await diagram.evaluate(node => {
      const surface = node.firstElementChild;
      const observer = new MutationObserver(() => {
        const measurement = surface.querySelector(':scope > div[id^="dnotist-mermaid-"]');
        if (!measurement) return;
        globalThis.measuredStyle = measurement.getAttribute('style');
        globalThis.measuredFamily = getComputedStyle(measurement).fontFamily;
        observer.disconnect();
      });
      observer.observe(surface, { childList: true });
      node.update();
    });
    await waitForSvg(beforeAncestry);
    await assertLabels();
    assert.equal(await diagram.locator('.nodeLabel p').first().evaluate(node => getComputedStyle(node).fontSize), '12px');
    assert.equal(await diagram.evaluate(node => {
      const svg = node.renderedSvg;
      return svg.parentElement.id === `d${svg.id}`
        && svg.parentElement.parentElement === node.firstElementChild
        && svg.parentElement.getAttribute('style') === globalThis.measuredStyle
        && getComputedStyle(svg.parentElement).fontFamily === globalThis.measuredFamily
        && !svg.parentElement.style.fontFamily.includes('undefined');
    }), true);
    const oldId = await diagram.locator('svg').getAttribute('id');
    await diagram.evaluate(node => {
      const root = node.shadowRoot;
      const surface = node.firstElementChild;
      node.remove();
      document.querySelector('main').append(node);
      globalThis.sameShadowRoot = node.shadowRoot === root && node.firstElementChild === surface;
    });
    await waitForSvg(oldId);
    assert.equal(await page.evaluate(() => globalThis.sameShadowRoot), true);
    await assertLabels();
    await assertTextVisible('解析 · 字体与布局');
    await assertTextVisible('返回内容 · Markup 模板与插值');
    const beforeRapidUpdates = await diagram.locator('svg').getAttribute('id');
    await diagram.evaluate(node => {
      const surface = node.firstElementChild;
      const observer = new MutationObserver(() => {
        const measurement = surface.querySelector(':scope > div[id^="dnotist-mermaid-"]');
        if (!measurement) return;
        observer.disconnect();
        node.setAttribute('notist-source', 'flowchart TB; A["Latest update"]');
        node.setAttribute('notist-theme', 'dark');
        globalThis.activeMeasurementPreserved = surface.contains(measurement);
      });
      observer.observe(surface, { childList: true });
      node.setAttribute('notist-source', 'flowchart TB; A["Discarded update"]');
    });
    await waitForSvg(beforeRapidUpdates);
    await page.waitForFunction(() =>
      document.querySelector('mermaid-diagram')?.renderedSvg?.textContent.includes('Latest update')
    );
    assert.equal(await page.evaluate(() => globalThis.activeMeasurementPreserved), true);
    assert.equal(await diagram.locator('.diagram').count(), 1);
    assert.equal(await diagram.locator('svg').count(), 1);
    assert.equal(await diagram.evaluate(node =>
      node.shadowRoot.querySelector('slot').assignedElements()[0] === node.firstElementChild
    ), true);
    await assertLabels();
    const beforeActiveReconnect = await diagram.locator('svg').getAttribute('id');
    await diagram.evaluate(node => {
      const surface = node.firstElementChild;
      const observer = new MutationObserver(() => {
        const measurement = surface.querySelector(':scope > div[id^="dnotist-mermaid-"]');
        if (!measurement) return;
        observer.disconnect();
        node.remove();
        document.querySelector('main').append(node);
        globalThis.reconnectMeasurementPreserved = surface.contains(measurement);
      });
      observer.observe(surface, { childList: true });
      node.setAttribute('notist-source', 'flowchart TB; A["Reconnect during render"]');
    });
    await waitForSvg(beforeActiveReconnect);
    assert.equal(await page.evaluate(() => globalThis.reconnectMeasurementPreserved), true);
    await assertTextVisible('Reconnect during render');
    await assertLabels();
    const configured = `%%{init: {"htmlLabels": true, "flowchart": {"htmlLabels": true}, "themeVariables": {"fontSize": "14px"}}}%%
      flowchart TB
        A["<b>第一行</b><br/>第二行"] --> B["带参数的内容函数"]
    `;
    const beforeConfigured = await diagram.locator('svg').getAttribute('id');
    await diagram.evaluate((node, source) => node.setAttribute('notist-source', source), configured);
    await waitForSvg(beforeConfigured);
    await assertLabels();
    await assertTextVisible('第一行第二行');
    assert.ok(await diagram.locator('foreignObject b, foreignObject strong').count() > 0);
    const beforeSvgLabels = await diagram.locator('svg').getAttribute('id');
    const svgConfigured = configured
      .replaceAll('"htmlLabels": true', '"htmlLabels": false')
      .replace('<b>第一行</b>', '第一行');
    await diagram.evaluate((node, source) => node.setAttribute('notist-source', source), svgConfigured);
    await waitForSvg(beforeSvgLabels);
    await assertLabels('svg');
    await assertTextVisible('第一行第二行');
    const beforeHtmlLabels = await diagram.locator('svg').getAttribute('id');
    await diagram.evaluate((node, source) => node.setAttribute('notist-source', source), configured);
    await waitForSvg(beforeHtmlLabels);
    await assertLabels();
    const fontResponse = await page.request.get('https://cdn.jsdelivr.net/npm/@fontsource/roboto-mono@5.1.1/files/roboto-mono-latin-400-normal.woff2');
    assert.ok(fontResponse.ok(), 'test font is available');
    const fontBody = await fontResponse.body();
    await page.route('**/mermaid-test-font.woff2*', async route => {
      // Deliberately finish after initial label layout, not on the first frame.
      await new Promise(resolve => setTimeout(resolve, 150));
      await route.fulfill({ contentType: 'font/woff2', body: fontBody });
    });
    await page.addStyleTag({ content: `
      @font-face {
        font-family: "MermaidLateFont";
        src: url("/mermaid-test-font.woff2?rich");
        font-weight: 700;
      }
    ` });
    const beforeLateFont = await diagram.locator('svg').getAttribute('id');
    await diagram.evaluate(node => {
      const surface = node.firstElementChild;
      globalThis.fontLayouts = 0;
      globalThis.fontLayoutObserver = new MutationObserver(records => {
        for (const record of records) {
          for (const child of record.addedNodes) {
            if (child.nodeType === Node.ELEMENT_NODE && child.querySelector('svg')) globalThis.fontLayouts++;
          }
        }
      });
      globalThis.fontLayoutObserver.observe(surface, { childList: true });
      node.setAttribute('notist-source', `flowchart TB
A["IIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIII"]:::late
classDef late font-family:MermaidLateFont,font-weight:700;`);
    });
    await waitForSvg(beforeLateFont);
    assert.equal(await page.evaluate(() => document.fonts.check('700 12px "MermaidLateFont"', 'I')), true);
    assert.ok(await page.evaluate(() => globalThis.fontLayouts) >= 3, 'late font causes a second layout before publishing');
    await page.evaluate(() => globalThis.fontLayoutObserver.disconnect());
    await assertLabels();
    await page.addStyleTag({ content: `
      @font-face { font-family: "MermaidConfiguredFont"; src: url("/mermaid-test-font.woff2?configured"); }
    ` });
    const beforeConfiguredFont = await diagram.locator('svg').getAttribute('id');
    await diagram.evaluate(node => {
      globalThis.configuredFontReadyAtLayout = false;
      const surface = node.firstElementChild;
      const observer = new MutationObserver(() => {
        if (!surface.querySelector(':scope > div[id^="dnotist-mermaid-"]')) return;
        globalThis.configuredFontReadyAtLayout = document.fonts.check('14px "MermaidConfiguredFont"', 'I');
        observer.disconnect();
      });
      observer.observe(surface, { childList: true });
      node.setAttribute('notist-source', `%%{init: {"fontFamily": "MermaidConfiguredFont", "themeVariables": {"fontSize": "14px"}}}%%
flowchart TB; A["IIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIII"]`);
    });
    await waitForSvg(beforeConfiguredFont);
    assert.equal(await page.evaluate(() => globalThis.configuredFontReadyAtLayout), true);
    await assertLabels();
    const beforeDark = await diagram.locator('svg').getAttribute('id');
    await diagram.evaluate(node => node.setAttribute('notist-theme', 'dark'));
    await waitForSvg(beforeDark);
    await assertLabels();
    await page.setViewportSize({ width: 390, height: 844 });
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    if (process.env.MERMAID_SCREENSHOT_DIR) {
      await mkdir(process.env.MERMAID_SCREENSHOT_DIR, { recursive: true });
      await diagram.screenshot({ path: join(process.env.MERMAID_SCREENSHOT_DIR, 'mermaid-labels.png') });
    }
    for (const [graph, visible] of [
      ['sequenceDiagram\nparticipant Reader\nparticipant Compiler\nReader->>Compiler: Content', 'Reader'],
      ['classDiagram\nclass Reader\nclass Book\nReader --> Book : Content', 'Reader'],
    ]) {
      const beforeOtherType = await diagram.locator('svg').getAttribute('id');
      await diagram.evaluate((node, source) => node.setAttribute('notist-source', source), graph);
      await waitForSvg(beforeOtherType);
      await assertTextVisible(visible);
      assert.equal(await diagram.locator('svg').count(), 1);
    }
    await diagram.evaluate(node => node.setAttribute('notist-source', 'flowchart TB\nA --> ['));
    await page.waitForFunction(() => document.querySelector('mermaid-diagram')?.textContent.includes('Diagram error:'));
    await diagram.evaluate((node, source) => node.setAttribute('notist-source', source), source);
    await waitForSvg();
    await assertLabels();
    assert.deepEqual(errors, []);
    console.log('Mermaid browser checks passed: HTML/SVG labels, host CSS, long text, subgraphs, reconnect, rapid updates, slots, directives, rich text, line breaks, dark theme, mobile and error recovery.');
  } finally {
    await browser.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
