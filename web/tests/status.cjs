// Build docs into target/status-preview, then serve the repository.
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
    const base = process.env.NOTIST_TEST_URL ?? 'http://127.0.0.1:8000';
    const path = process.env.NOTIST_STATUS_PAGE ?? '/target/status-preview/components/';
    const page = await browser.newPage({ viewport: { width: 1280, height: 960 }, ignoreHTTPSErrors: true });
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(`${base}${path}`);
    await page.waitForFunction(() => document.querySelector('[notist-function*=mermaid]')?.renderedSvg, null, { timeout: 90000 });
    for (const [status, label] of [['draft', 'Draft · 草案'], ['staged', 'Staged · 待实现'], ['outdate', 'Outdate · 已过时']]) {
      const section = page.locator(`div[data-status="${status}"]`);
      assert.equal(await section.count(), 1);
      assert.equal(await section.evaluate(node => node.shadowRoot), null);
      assert.equal(await section.evaluate(node => getComputedStyle(node).borderTopStyle), status === 'draft' ? 'dashed' : 'solid');
      assert.equal(await section.evaluate(node => getComputedStyle(node, '::before').content), `"${label}"`);
      assert.equal(await section.evaluate(node => {
        const paragraph = node.querySelector('p');
        const typography = () => {
          const style = getComputedStyle(paragraph);
          return [style.fontFamily, style.fontSize, style.fontWeight, style.lineHeight, style.letterSpacing].join('|');
        };
        const before = typography();
        const status = node.dataset.status;
        delete node.dataset.status;
        const withoutStatus = typography();
        node.dataset.status = status;
        return before === withoutStatus;
      }), true);
    }
    assert.equal(await page.locator('div[data-status="draft"] [notist-function$=badge][notist-label="Draft"]').count(), 1);
    const inline = page.locator('span[data-status="outdate"]');
    assert.equal(await inline.count(), 1);
    assert.equal(await inline.evaluate(node => getComputedStyle(node).display), 'inline');
    assert.equal(await inline.evaluate(node => getComputedStyle(node).textDecorationStyle), 'wavy');
    assert.equal(await inline.evaluate(node => getComputedStyle(node).textDecorationThickness), '1px');
    const cells = page.locator('td[data-status="outdate"]');
    assert.equal(await cells.count(), 2);
    for (const cell of await cells.all()) {
      assert.deepEqual(await cell.evaluate(node => {
        const style = getComputedStyle(node, '::before');
        return [style.top, style.right, style.bottom, style.left, style.pointerEvents];
      }), ['4px', '6px', '4px', '6px', 'none']);
    }
    const colors = [];
    for (const theme of ['light', 'dark']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      colors.push(await page.locator('div[data-status="outdate"]').evaluate(node => getComputedStyle(node).borderTopColor));
      if (process.env.STATUS_SCREENSHOT_DIR) {
        await mkdir(process.env.STATUS_SCREENSHOT_DIR, { recursive: true });
        await page.addStyleTag({ content: '.site-header { position: static !important; }' });
        for (const status of ['draft', 'staged', 'outdate']) {
          await page.locator(`div[data-status="${status}"]`).screenshot({ path: join(process.env.STATUS_SCREENSHOT_DIR, `${status}-${theme}.png`) });
        }
      }
    }
    assert.notEqual(colors[0], colors[1]);
    // Changing metadata immediately changes presentation, without a component lifecycle.
    const changed = await page.evaluate(() => {
      const node = document.createElement('section');
      node.textContent = 'Dynamic state';
      document.body.append(node);
      const labels = ['draft', 'staged', 'outdate'].map(status => {
        node.dataset.status = status;
        return getComputedStyle(node, '::before').content;
      });
      node.remove();
      return labels;
    });
    assert.deepEqual(changed, ['"Draft · 草案"', '"Staged · 待实现"', '"Outdate · 已过时"']);
    await page.setViewportSize({ width: 390, height: 844 });
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    await page.emulateMedia({ media: 'print' });
    const draft = page.locator('div[data-status="draft"]');
    assert.equal(await draft.evaluate(node => getComputedStyle(node).borderTopStyle), 'solid');
    assert.equal(await draft.evaluate(node => getComputedStyle(node).backgroundColor), 'rgba(0, 0, 0, 0)');
    assert.equal(await cells.first().evaluate(node => getComputedStyle(node, '::before').backgroundColor), 'rgba(0, 0, 0, 0)');
    await page.emulateMedia({ media: 'screen' });
    // The staged pipeline keeps its overview heading outside the status frame.
    await page.goto(`${base}${process.env.NOTIST_PIPELINE_PAGE ?? '/target/status-preview/designs/pipeline/markup/'}`);
    const design = page.locator('div[data-status="staged"]').filter({ has: page.locator('[notist-function*=mermaid]') });
    assert.equal(await design.count(), 1);
    assert.equal(await design.evaluate(node => node.previousElementSibling?.tagName === 'H2'
      && node.previousElementSibling.textContent === '数据流'), true);
    assert.match(await design.textContent(), /Item 森林/);
    assert.equal(await design.locator('[notist-function*=mermaid]').count(), 1);
    await page.waitForFunction(() => document.querySelector('div[data-status="staged"] [notist-function*=mermaid]')?.renderedSvg, null, { timeout: 90000 });
    assert.equal(await design.locator('[notist-function*=mermaid]').evaluate(node => /Evaluate|Renderer|--eval/.test(node.renderedSvg.textContent)), false);
    assert.deepEqual(errors, []);
    const fallback = await browser.newPage({ javaScriptEnabled: false });
    await fallback.goto(`${base}${path}`);
    for (const status of ['draft', 'staged', 'outdate']) {
      const section = fallback.locator(`div[data-status="${status}"]`);
      await section.waitFor({ state: 'visible' });
      assert.notEqual(await section.evaluate(node => getComputedStyle(node, '::before').content), 'none');
    }
    assert.equal(await fallback.locator('span[data-status="outdate"]').evaluate(node => getComputedStyle(node).textDecorationStyle), 'wavy');
    console.log('Status checks passed: annotations, nested content, inline and cell styles, themes, mobile, print, diagrams and no-JS presentation.');
  } finally {
    await browser.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
