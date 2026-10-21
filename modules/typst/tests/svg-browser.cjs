// Export modules/typst/README.not and serve the repository before running this suite.
const assert = require("node:assert/strict");
const { chromium } = require("playwright");

(async () => {
  const browser = await chromium.launch({
    headless: true,
    ...(process.env.NOTIST_CHROMIUM ? { executablePath: process.env.NOTIST_CHROMIUM } : {}),
  });
  try {
    const base = process.env.NOTIST_TEST_URL ?? "http://127.0.0.1:8000";
    const context = await browser.newContext({ viewport: { width: 1200, height: 900 } });
    const requests = [];
    context.on("request", request => requests.push(request.url()));
    const page = await context.newPage();
    page.setDefaultTimeout(90000);
    const errors = [];
    const workers = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("worker", worker => workers.push(worker));
    const demo = process.env.NOTIST_TYPST_DEMO ?? "/target/typst-demo/";
    const selector = "[notist-function$='.svg']";
    await page.goto(`${base}${demo}`);
    const rendered = () => page.waitForFunction(selector => {
      const nodes = [...document.querySelectorAll(selector)];
      return nodes.length === 2 && nodes.every(node =>
        node.renderedSvg?.getAttribute("aria-label") === node.getAttribute("notist-source")
        && !node.shadowRoot.querySelector(".error").textContent);
    }, selector);
    await rendered();
    assert.equal(workers.length, 1, "SVG components share a compilation worker");
    assert.equal(await page.locator(`${selector} svg script, ${selector} svg foreignObject`).count(), 0);
    assert.ok(await page.locator(`${selector} svg path`).count() > 0);
    assert.equal(await page.locator(selector).evaluateAll(nodes => nodes.every(node => !node.closest("p"))), true);
    const packageRequests = () => requests.filter(url => url.startsWith("https://packages.typst.org/preview/"));
    assert.ok(packageRequests().some(url => url.endsWith("fletcher-0.5.8.tar.gz")), "Fletcher is loaded at runtime");
    assert.ok(packageRequests().some(url => /cetz-[\d.]+\.tar\.gz$/.test(url)), "transitive CeTZ dependency is loaded");
    const packages = packageRequests().slice();
    assert.equal(new Set(packages).size, packages.length, "each package version is fetched once");
    const font = "NotoSerifCJKsc-Regular.otf";
    assert.equal(requests.filter(url => url.endsWith(font)).length, 1);
    assert.equal(requests.filter(url => url.endsWith("LibertinusSerif-Regular.otf")).length, 1);
    const drawing = page.locator(selector).nth(1);
    const original = await drawing.getAttribute("notist-source");
    const originalSvg = await drawing.locator("svg").evaluate(node => node.outerHTML);
    await drawing.evaluate(node => node.setAttribute("notist-source", node.getAttribute("notist-source") + "\n#v(3pt)\n第二次绘图"));
    await rendered();
    assert.notEqual(await drawing.locator("svg").evaluate(node => node.outerHTML), originalSvg);
    assert.deepEqual(packageRequests(), packages, "subsequent compilation reuses package cache");

    // A user-controlled page size and multiple pages override the drawing defaults.
    const general = page.locator(selector).first();
    await general.evaluate(node => node.setAttribute("notist-source", '#set page(width: 90pt, height: 60pt, margin: 4pt)\n第一页\n#pagebreak()\n第二页'));
    await rendered();
    const dimensions = await general.locator("svg").evaluate(svg => ({
      width: svg.viewBox.baseVal.width,
      height: svg.viewBox.baseVal.height,
      pages: svg.querySelectorAll(".typst-page").length,
    }));
    assert.equal(dimensions.width, 90);
    assert.ok(dimensions.height >= 120, JSON.stringify(dimensions));
    assert.equal(dimensions.pages, 2);

    // Errors clear the current result, preserve source, and leave the worker usable.
    await drawing.evaluate(node => node.setAttribute("notist-source", "#rect("));
    assert.equal(await drawing.evaluate(node => node.renderedSvg), null);
    await page.waitForFunction(selector => document.querySelectorAll(selector)[1].shadowRoot.querySelector(".error").textContent.includes("unclosed"), selector);
    assert.equal(await drawing.locator(".source").textContent(), "#rect(");
    await general.evaluate(node => node.setAttribute("notist-source", "另一个组件继续编译 $x^2$"));
    await page.waitForFunction(selector => document.querySelector(selector).renderedSvg !== null, selector);
    await drawing.evaluate((node, original) => {
      node.setAttribute("notist-source", "#missing-name");
      node.setAttribute("notist-source", "old input");
      node.setAttribute("notist-source", original);
    }, original);
    await rendered();
    assert.equal(await drawing.locator("svg").getAttribute("aria-label"), original);
    assert.deepEqual(packageRequests(), packages);
    await context.route("https://packages.typst.org/preview/notist-missing-test-0.0.0.tar.gz", route => route.fulfill({ status: 404, body: "not found" }));
    await drawing.evaluate(node => node.setAttribute("notist-source", '#import "@preview/notist-missing-test:0.0.0": *'));
    await page.waitForFunction(selector => document.querySelectorAll(selector)[1].shadowRoot.querySelector(".error").textContent.includes("package"), selector);
    assert.equal(await drawing.evaluate(node => node.renderedSvg), null);
    await drawing.evaluate((node, original) => node.setAttribute("notist-source", original), original);
    await rendered();
    assert.equal(packageRequests().filter(url => url.endsWith("fletcher-0.5.8.tar.gz")).length, 1, "a missing package does not discard successful packages");
    assert.equal(await drawing.evaluate(node => {
      const parent = node.parentNode;
      const next = node.nextSibling;
      node.remove();
      const unavailable = node.renderedSvg === null;
      node.setAttribute("notist-source", '重连后 $sqrt(2)$');
      parent.insertBefore(node, next);
      return unavailable;
    }), true);
    await rendered();
    assert.equal(workers.length, 1);
    await drawing.evaluate(node => node.setAttribute("notist-source", ""));
    assert.equal(await drawing.evaluate(node => node.renderedSvg), null);
    assert.equal(await drawing.locator(".source").textContent(), "");
    await drawing.evaluate((node, original) => node.setAttribute("notist-source", original), original);
    await rendered();
    await drawing.screenshot({ path: `.${demo}fletcher.png` });
    await general.evaluate(node => node.setAttribute("notist-source", '#rect(width: 900pt, height: 150pt)[宽图]'));
    await rendered();
    await page.setViewportSize({ width: 390, height: 844 });
    const responsive = await general.locator("svg").evaluate(svg => {
      const host = svg.getRootNode().host;
      return svg.getBoundingClientRect().width <= host.getBoundingClientRect().width + 1;
    });
    assert.ok(responsive);

    // The Worker-based Notist preview discovers the native svg entry for .not and .md.
    await page.goto(`${base}/web/`);
    await page.waitForSelector("#core .kind");
    await page.fill("#root-url", "../modules/typst/");
    await page.fill("#config-url", "");
    await page.fill("#path", "README.not");
    await page.click("#load-project");
    const preview = () => page.waitForFunction(() => {
      const doc = document.querySelector("#preview").contentDocument;
      const nodes = [...(doc?.querySelectorAll("[notist-function$='.svg']") ?? [])];
      return nodes.length > 0 && nodes.every(node => node.renderedSvg?.getAttribute("aria-label") === node.getAttribute("notist-source"));
    });
    await preview();
    assert.equal(await page.locator("#diags").textContent(), "✓ 无诊断");
    await page.fill("#path", "drawing.md");
    await page.locator("#path").dispatchEvent("change");
    await page.fill("#src", '#import "lib.notc" as typst;\n#typst.svg("中文 $sqrt(2)$")');
    await preview();
    assert.equal(await page.frameLocator("#preview").locator(selector).count(), 1);
    assert.equal(await page.locator("#diags").textContent(), "✓ 无诊断");
    assert.deepEqual(errors, []);
    console.log("Typst SVG: published worker, Chinese text, Fletcher runtime packages/cache, pages, lifecycle, errors, responsive sizing and .not/.md preview passed.");
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
