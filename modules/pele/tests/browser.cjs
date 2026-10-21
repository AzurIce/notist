// Generate target/pele-demo and serve the repository root before running.
const assert = require("node:assert/strict");
const fs = require("node:fs/promises");
const path = require("node:path");
const { chromium } = require("playwright");

(async () => {
  const browser = await chromium.launch({ headless: true, ...(process.env.NOTIST_CHROMIUM ? { executablePath: process.env.NOTIST_CHROMIUM } : {}) });
  try {
    const base = process.env.NOTIST_TEST_URL ?? "http://127.0.0.1:8000";
    const page = await browser.newPage({ viewport: { width: 1200, height: 900 } });
    const errors = [];
    const libraries = [];
    const selector = '[notist-function$="/pele/lib.notc.diagram"]';
    page.on("pageerror", error => errors.push(error.message));
    page.on("request", request => {
      if (request.url().includes("/npm/pele@")) libraries.push(request.url());
    });
    await page.goto(`${base}${process.env.NOTIST_PELE_DEMO ?? "/target/pele-demo/"}`);
    await page.waitForFunction(selector => {
      const components = [...document.querySelectorAll(selector)];
      return components.length === 2 && components.every(element => element.renderedSvg);
    }, selector);
    assert.deepEqual(libraries, ["https://cdn.jsdelivr.net/npm/pele@1.6.0/dist/pele.js"], "instances share the pinned library");
    assert.equal(await page.locator(`${selector} foreignObject`).count(), 0);
    assert.equal(await page.locator(selector).first().getAttribute("notist-theme"), "auto", "Notist applies the signature default");
    await page.evaluate(selector => { window.diagram = document.querySelector(selector); }, selector);

    // Long CJK labels and nested directions remain inside their measured shapes.
    await page.evaluate(() => {
      diagram.setAttribute("notist-source", `flowchart LR
        outside[入口] --> first
        subgraph stack[共享管线]
          direction TB
          first[中文与 English 的长标签] --> second[下一阶段]
        end
        second --> result([Item 树])`);
    });
    await page.waitForFunction(() => diagram.renderedSvg?.textContent.includes("中文与 English"));
    const geometry = await page.evaluate(() => {
      const svg = diagram.renderedSvg;
      const center = id => {
        const matrix = svg.querySelector(`.pele-node[data-id="${id}"]`).transform.baseVal.consolidate().matrix;
        return { x: matrix.e, y: matrix.f };
      };
      const clipped = [...svg.querySelectorAll(".pele-node")].flatMap(node => {
        const label = node.querySelector("text");
        const rect = node.querySelector("rect");
        if (!label || !rect) return [];
        const text = label.getBBox(), box = rect.getBBox();
        return text.x < box.x - 2 || text.y < box.y - 2 || text.x + text.width > box.x + box.width + 2 || text.y + text.height > box.y + box.height + 2
          ? [label.textContent] : [];
      });
      return { first: center("first"), second: center("second"), clipped };
    });
    assert.ok(Math.abs(geometry.first.x - geometry.second.x) < 1, "subgraph direction survives external edges");
    assert.ok(geometry.first.y < geometry.second.y);
    assert.deepEqual(geometry.clipped, []);
    const labelFill = () => page.evaluate(() => getComputedStyle(diagram.renderedSvg.querySelector(".pele-label")).fill);

    // Host tag selectors cannot affect the shadow SVG; CSS variables can.
    await page.addStyleTag({ content: 'svg { font-size: 80px !important; } text { fill: red !important; } .pele-node rect { width: 1px !important; }' });
    await page.evaluate(() => {
      window.originalSvg = diagram.renderedSvg;
      document.documentElement.style.setProperty("--fg", "rgb(32, 48, 64)");
    });
    assert.equal(await labelFill(), "rgb(32, 48, 64)");
    await page.evaluate(() => diagram.setAttribute("notist-theme", "dark"));
    assert.equal(await labelFill(), "rgb(222, 223, 213)");
    await page.evaluate(() => diagram.setAttribute("notist-theme", "light"));
    assert.equal(await labelFill(), "rgb(48, 51, 47)");
    await page.evaluate(() => diagram.setAttribute("notist-theme", "unknown"));
    assert.equal(await labelFill(), "rgb(32, 48, 64)");
    await page.evaluate(() => diagram.style.setProperty("--pele-fg", "rgb(15, 90, 65)"));
    assert.equal(await labelFill(), "rgb(15, 90, 65)");
    assert.ok(await page.evaluate(() => diagram.renderedSvg === originalSvg), "color changes do not redraw");
    await page.evaluate(() => {
      diagram.style.removeProperty("--pele-fg");
      document.documentElement.style.removeProperty("--fg");
      diagram.setAttribute("notist-theme", "auto");
      diagram.setAttribute("notist-source", "flowchart LR; Old --> Stale");
      diagram.setAttribute("notist-source", "flowchart LR; Current --> Latest");
    });
    await page.waitForFunction(() => diagram.renderedSvg?.textContent.includes("Latest"));
    assert.ok(!(await page.evaluate(() => diagram.renderedSvg.textContent)).includes("Stale"));

    // A deferred update must not expose the previous SVG as the current result.
    await page.evaluate(() => {
      diagram.style.display = "none";
      diagram.setAttribute("notist-source", "flowchart LR; Hidden --> Revealed");
    });
    await page.evaluate(() => Promise.resolve());
    assert.ok(await page.evaluate(() => diagram.renderedSvg === null));
    await page.evaluate(() => { diagram.style.display = ""; });
    await page.waitForFunction(() => diagram.renderedSvg?.textContent.includes("Revealed"));

    // Resize switches a simple horizontal graph to vertical without page overflow.
    await page.evaluate(() => diagram.setAttribute("notist-source", "flowchart LR; A[Parse / Lower] --> B[Resolve] --> C[Shape] --> D[Materialize] --> E[Evaluate] --> F[Final Item]"));
    await page.waitForFunction(() => diagram.renderedSvg?.textContent.includes("Final Item"));
    await page.setViewportSize({ width: 390, height: 900 });
    await page.waitForFunction(() => {
      const box = diagram.renderedSvg?.getBoundingClientRect();
      return box && box.height > box.width;
    });
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth));
    // A wide graph that cannot reflow offers functional zoom controls.
    await page.evaluate(() => diagram.setAttribute("notist-source", `flowchart TB
      root --> a[Long label alpha]
      root --> b[Long label beta]
      root --> c[Long label gamma]
      root --> d[Long label delta]`));
    await page.waitForFunction(() => diagram.renderedSvg?.textContent.includes("gamma"));
    const zoom = page.locator(selector).first().getByRole("button", { name: "Zoom in", exact: true });
    await zoom.waitFor({ state: "visible" });
    const fittedViewBox = await page.evaluate(() => diagram.renderedSvg.getAttribute("viewBox"));
    await zoom.click();
    assert.notEqual(await page.evaluate(() => diagram.renderedSvg.getAttribute("viewBox")), fittedViewBox);
    await page.locator(selector).first().getByRole("button", { name: "Reset zoom", exact: true }).click();
    assert.equal(await page.evaluate(() => diagram.renderedSvg.getAttribute("viewBox")), fittedViewBox);

    // Invalid source invalidates renderedSvg, retains the last good preview, and recovers.
    await page.evaluate(() => {
      window.goodSvg = diagram.renderedSvg;
      diagram.setAttribute("notist-source", "flowchart TB; Broken[");
    });
    await page.locator(selector).first().locator(".pele-error").waitFor();
    assert.ok(await page.evaluate(() => diagram.renderedSvg === null && diagram.shadowRoot.querySelector("svg") === goodSvg));
    assert.equal(await page.evaluate(() => goodSvg.style.opacity), "0.4");
    await page.evaluate(() => diagram.setAttribute("notist-source", 'flowchart TB; Safe["<img src=x onerror=alert(1)>"] --> Recovered'));
    await page.waitForFunction(() => diagram.renderedSvg?.textContent.includes("Recovered"));
    assert.equal(await page.locator(`${selector} .pele-error`).count(), 0);
    assert.equal(await page.locator(`${selector} img, ${selector} script, ${selector} foreignObject`).count(), 0);

    // A second diagram has independent state; reconnect reuses the shadow container.
    await page.evaluate(() => {
      const sibling = diagram.cloneNode(false);
      sibling.id = "independent";
      sibling.setAttribute("notist-source", "sequenceDiagram\nAlice->>Bob: Independent");
      diagram.after(sibling);
      window.surface = diagram.shadowRoot.querySelector(".diagram");
      const parent = diagram.parentNode, next = diagram.nextSibling;
      diagram.remove();
      window.disconnectedSvg = diagram.renderedSvg;
      diagram.setAttribute("notist-source", "flowchart TB; Reconnected --> Ready");
      parent.insertBefore(diagram, next);
    });
    await page.waitForFunction(() => diagram.renderedSvg?.textContent.includes("Ready") && document.getElementById("independent").renderedSvg?.textContent.includes("Independent"));
    assert.ok(await page.evaluate(() => disconnectedSvg === null && diagram.shadowRoot.querySelector(".diagram") === surface && diagram.shadowRoot.querySelectorAll(".diagram").length === 1));
    assert.deepEqual(await page.evaluate(() => {
      const ids = [diagram, document.getElementById("independent")].flatMap(element => [...element.renderedSvg.querySelectorAll("[id]")].map(node => node.id));
      return ids.filter((id, index) => ids.indexOf(id) !== index);
    }), []);

    // The browser host loads the same source import and directory component.
    const preview = await browser.newPage();
    preview.on("pageerror", error => errors.push(error.message));
    await preview.goto(`${base}/web/`);
    await preview.waitForSelector("#core .kind");
    await preview.fill("#root-url", "../modules/pele/");
    await preview.fill("#config-url", "");
    await preview.fill("#path", "README.not");
    await preview.click("#load-project");
    await preview.waitForFunction(() => {
      const elements = [...(document.querySelector("#preview").contentDocument?.querySelectorAll('[notist-function$="/pele/lib.notc.diagram"]') ?? [])];
      return elements.length === 2 && elements.every(element => element.renderedSvg);
    });
    assert.equal(await preview.locator("#diags").textContent(), "✓ 无诊断");
    await preview.close();

    // Test loading separately: a detached element cannot mount a stale request.
    const pending = await browser.newPage();
    pending.on("pageerror", error => errors.push(error.message));
    let release;
    const gate = new Promise(resolve => { release = resolve; });
    await pending.route("https://cdn.jsdelivr.net/npm/pele@1.6.0/dist/pele.js", async route => {
      await gate;
      await route.continue();
    });
    await pending.goto(`${base}/modules/pele/`);
    await pending.evaluate(async () => {
      const { default: Diagram } = await import("./diagram/index.js");
      customElements.define("pele-test", Diagram);
      window.element = document.createElement("pele-test");
      element.setAttribute("notist-source", "flowchart LR; Stale --> Detached");
      document.body.append(element);
    });
    await pending.evaluate(() => element.remove());
    release();
    await pending.evaluate(() => import("https://cdn.jsdelivr.net/npm/pele@1.6.0/dist/pele.js"));
    assert.ok(await pending.evaluate(() => element.renderedSvg === null && !element.shadowRoot.querySelector("svg")));
    await pending.evaluate(() => {
      element.setAttribute("notist-source", "flowchart LR; Latest --> Connected");
      document.body.append(element);
    });
    await pending.waitForFunction(() => element.renderedSvg?.textContent.includes("Connected"));
    // Without site palette variables, auto still follows inherited color-scheme.
    await pending.evaluate(() => {
      window.themeSvg = element.renderedSvg;
      document.documentElement.style.colorScheme = "light";
    });
    await pending.emulateMedia({ colorScheme: "dark" });
    await pending.waitForFunction(() => element.dataset.peleScheme === "light");
    assert.equal(await pending.evaluate(() => getComputedStyle(element.renderedSvg.querySelector(".pele-label")).fill), "rgb(48, 51, 47)");
    await pending.evaluate(() => { document.documentElement.style.colorScheme = "dark"; });
    await pending.emulateMedia({ colorScheme: "light" });
    await pending.waitForFunction(() => element.dataset.peleScheme === "dark");
    assert.equal(await pending.evaluate(() => getComputedStyle(element.renderedSvg.querySelector(".pele-label")).fill), "rgb(222, 223, 213)");
    await pending.evaluate(() => { document.documentElement.style.colorScheme = ""; });
    await pending.waitForFunction(() => element.dataset.peleScheme === "light");
    await pending.emulateMedia({ colorScheme: "dark" });
    await pending.waitForFunction(() => element.dataset.peleScheme === "dark");
    assert.ok(await pending.evaluate(() => element.renderedSvg === themeSvg), "theme updates preserve the SVG");
    await pending.close();

    const failed = await browser.newPage();
    failed.on("pageerror", error => errors.push(error.message));
    await failed.route("https://cdn.jsdelivr.net/npm/pele@1.6.0/dist/pele.js", route => route.abort());
    await failed.goto(`${base}/target/pele-demo/`);
    await failed.locator(selector).first().getByRole("alert").waitFor();
    assert.ok(await failed.locator(selector).first().evaluate(element => element.renderedSvg === null));
    await failed.close();

    if (process.env.PELE_SCREENSHOT_DIR) {
      await fs.mkdir(process.env.PELE_SCREENSHOT_DIR, { recursive: true });
      await page.setViewportSize({ width: 1200, height: 900 });
      await page.reload();
      await page.waitForFunction(selector => [...document.querySelectorAll(selector)].every(element => element.renderedSvg), selector);
      await page.evaluate(() => { document.activeElement?.blur(); window.scrollTo(0, 0); });
      await page.screenshot({ path: path.join(process.env.PELE_SCREENSHOT_DIR, "pele-desktop.png"), fullPage: true });
      await page.locator(selector).nth(1).screenshot({ path: path.join(process.env.PELE_SCREENSHOT_DIR, "pele-diagram.png") });
      await page.setViewportSize({ width: 390, height: 900 });
      await page.waitForFunction(selector => {
        const box = document.querySelectorAll(selector)[1].renderedSvg.getBoundingClientRect();
        return box.height > box.width;
      }, selector);
      await page.evaluate(() => { document.activeElement?.blur(); window.scrollTo(0, 0); });
      await page.screenshot({ path: path.join(process.env.PELE_SCREENSHOT_DIR, "pele-mobile.png"), fullPage: true });
    }
    assert.deepEqual(errors, []);
    console.log("Pele browser checks passed");
  } finally {
    await browser.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
