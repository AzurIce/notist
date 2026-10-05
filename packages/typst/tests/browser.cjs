// Export packages/typst/README.not to target/typst-demo and serve the repository.
const assert = require("node:assert/strict");
const { chromium } = require("playwright");

(async () => {
  const browser = await chromium.launch({
    headless: true,
    ...(process.env.NOTIST_CHROMIUM ? { executablePath: process.env.NOTIST_CHROMIUM } : {}),
  });
  try {
    const base = process.env.NOTIST_TEST_URL ?? "http://127.0.0.1:8000";
    const page = await browser.newPage({ viewport: { width: 1200, height: 900 } });
    page.setDefaultTimeout(90000);
    const errors = [];
    const requests = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("request", request => requests.push(request.url()));
    await page.goto(`${base}/target/typst-demo/`);
    const rendered = () => page.waitForFunction(() => {
      const nodes = [...document.querySelectorAll("typst-math")];
      return nodes.length === 4 && nodes.every(node =>
        node.shadowRoot?.querySelector("svg")?.getAttribute("aria-label") === node.getAttribute("notist-text")
        && !node.shadowRoot.querySelector(".error").textContent);
    }, undefined, { timeout: 90000 });
    await rendered();
    assert.equal(requests.filter(url => url.endsWith("typst_ts_web_compiler_bg.wasm")).length, 1);
    assert.equal(requests.filter(url => url.endsWith("typst_ts_renderer_bg.wasm")).length, 1);
    assert.equal(requests.filter(url => url.endsWith("NewCMMath-Regular.otf")).length, 1);
    assert.equal(requests.filter(url => url.endsWith("NewCM10-Regular.otf")).length, 1);
    assert.equal(await page.locator("typst-math svg path").count() > 0, true);
    assert.equal(await page.locator("typst-math svg script, typst-math svg foreignObject").count(), 0);
    const originals = await page.locator("typst-math svg").evaluateAll(nodes => nodes.map(node => node.outerHTML));
    assert.equal(new Set(originals).size, 4, "each queued formula produces its own SVG");
    const geometry = await page.locator("typst-math svg").evaluateAll(nodes => nodes.map(svg => {
      const box = svg.getBBox();
      const view = svg.viewBox.baseVal;
      return {
        width: svg.getBoundingClientRect().width,
        height: svg.getBoundingClientRect().height,
        baseline: parseFloat(svg.style.verticalAlign),
        containsInk: view.y <= box.y && view.y + view.height >= box.y + box.height,
      };
    }));
    assert.ok(geometry.every(item => item.width > 0 && item.height > 0 && Number.isFinite(item.baseline) && item.containsInk));
    await page.screenshot({ path: "target/typst-demo/preview.png", fullPage: true });

    const formula = page.locator("typst-math").first();
    const source = '"<a & b>" + alpha';
    await formula.evaluate((node, source) => {
      node.setAttribute("notist-text", "frac(");
      node.setAttribute("notist-text", "x^2");
      node.setAttribute("notist-text", source);
    }, source);
    await rendered();
    assert.equal(await formula.locator("svg").getAttribute("aria-label"), source);
    const updated = await formula.locator("svg").evaluate(node => node.outerHTML);
    assert.notEqual(updated, originals[0]);
    assert.equal(await formula.evaluate(node => {
      const root = node.shadowRoot;
      const parent = node.parentNode;
      const next = node.nextSibling;
      node.remove();
      node.setAttribute("notist-text", "sqrt(2)");
      parent.insertBefore(node, next);
      return node.shadowRoot === root;
    }), true);
    await rendered();
    assert.equal(await formula.locator("svg").getAttribute("aria-label"), "sqrt(2)");

    await formula.evaluate(node => node.setAttribute("notist-text", "frac("));
    await page.waitForFunction(() => document.querySelector("typst-math").shadowRoot.querySelector(".error").textContent.includes("unclosed delimiter"));
    assert.equal(await formula.locator(".formula").textContent(), "frac(");
    assert.equal(await formula.locator("svg").count(), 0);
    // A rejected compile must release the queue for other components.
    await page.locator("typst-math").nth(1).evaluate(node => node.setAttribute("notist-text", "x + y"));
    await page.waitForFunction(() => document.querySelectorAll("typst-math")[1].shadowRoot.querySelector("svg")?.getAttribute("aria-label") === "x + y");
    await formula.evaluate(node => node.removeAttribute("notist-text"));
    assert.equal(await formula.locator(".formula").textContent(), "");
    assert.equal(await formula.locator(".error").textContent(), "");
    await formula.evaluate(node => node.setAttribute("notist-text", "frac(1, 2)"));
    await rendered();

    const size = await formula.locator("svg").evaluate(svg => svg.getBoundingClientRect().width);
    await formula.evaluate(node => node.style.fontSize = "32px");
    const scaled = await formula.locator("svg").evaluate(svg => svg.getBoundingClientRect().width);
    assert.ok(scaled > size * 1.5, "inline math follows inherited font size");

    // The existing Worker pipeline can replace native math with this package.
    await page.goto(`${base}/web/`);
    await page.waitForSelector("#core .kind");
    await page.fill("#config-url", "../packages/typst/Notist.toml");
    await page.fill("#path", "README.not");
    await page.click("#load-project");
    const preview = count => page.waitForFunction(count => {
      const doc = document.querySelector("#preview").contentDocument;
      const nodes = [...(doc?.querySelectorAll("typst-math") ?? [])];
      return nodes.length === count && nodes.every(node =>
        node.shadowRoot?.querySelector("svg")?.getAttribute("aria-label") === node.getAttribute("notist-text"));
    }, count, { timeout: 90000 });
    await preview(4);
    assert.match(await page.locator("#core").textContent(), /Math/);
    assert.equal(await page.locator("#diags").textContent(), "✓ 无诊断");
    await page.fill("#path", "math.md");
    await page.locator("#path").dispatchEvent("change");
    await page.fill("#src", "$frac(a, b)$ and $sqrt(x)$ and $alpha + beta$");
    await preview(3);
    assert.equal(await page.locator("#diags").textContent(), "✓ 无诊断");
    assert.deepEqual(errors, []);
    console.log("Typst SVG, concurrency, lifecycle, errors, scaling and .not/.md Worker transforms passed.");
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
