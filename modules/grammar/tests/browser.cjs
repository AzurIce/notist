// Build the module and target/grammar-demo, then serve the repository root.
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
    const requests = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("request", request => requests.push(request.url()));
    await page.goto(`${base}${process.env.NOTIST_GRAMMAR_DEMO ?? "/target/grammar-demo/"}`);
    await page.waitForFunction(() => {
      const components = [...document.querySelectorAll("[notist-function*=grammar]")];
      return components.length === 6 && components.every(component => component.shadowRoot?.querySelector("svg"));
    });
    assert.equal(await page.locator("[notist-function*=grammar] svg").count(), 17);
    assert.ok(requests.some(url => url.endsWith("grammar_bg.wasm")), "loaded the published WASM asset");
    assert.ok(requests.every(url => url.startsWith(base)), "works without CDN dependencies");

    const link = page.locator("[notist-function*=grammar]").first().locator("svg a").first();
    await link.click();
    assert.ok(page.url() === `${base}${process.env.NOTIST_GRAMMAR_DEMO ?? "/target/grammar-demo/"}`, "rule navigation stays within the shadow root");

    for (const component of await page.locator("[notist-function*=grammar]").all()) {
      // The source block is host-rendered raw content beside the component.
      const host = component.locator("xpath=..");
      assert.equal(await host.locator("pre code").first().textContent(), await component.getAttribute("notist-source"));
    }

    const original = await page.locator("[notist-function*=grammar]").first().getAttribute("notist-source");
    const title = () => page.locator("[notist-function*=grammar]").first().locator("svg title").first();
    await page.evaluate(() => {
      const component = document.querySelector("[notist-function*=grammar]");
      component.setAttribute("notist-source", 'syntax Updated -> `new`;');
      component.setAttribute("notist-rule", "Updated");
      component.setAttribute("notist-theme", "navy");
    });
    await page.waitForFunction(() => document.querySelector("[notist-function*=grammar]").shadowRoot.querySelector("svg title")?.textContent.trim() === "syntax Updated");
    assert.equal((await title().textContent()).trim(), "syntax Updated");

    assert.equal(await page.evaluate(() => {
      const component = document.querySelector("[notist-function*=grammar]");
      const root = component.shadowRoot;
      const next = component.nextSibling;
      const parent = component.parentNode;
      component.remove();
      parent.insertBefore(component, next);
      return component.shadowRoot === root;
    }), true);
    await page.waitForFunction(() => document.querySelector("[notist-function*=grammar]").shadowRoot.querySelector("svg title")?.textContent.trim() === "syntax Updated");

    await page.evaluate(() => {
      const component = document.querySelector("[notist-function*=grammar]");
      component.removeAttribute("notist-rule");
      component.setAttribute("notist-source", "syntax Broken -> (");
    });
    await page.waitForSelector("[notist-function*=grammar] .error");
    const diagnostic = await page.locator("[notist-function*=grammar] .error").textContent();
    assert.match(diagnostic, /1:\d+: expected an expression/);
    // Changing the graph's parameters does not re-evaluate diagram's raw sibling.
    const sourceBlock = page.locator("[notist-function*=grammar]").first().locator("xpath=..").locator("pre code");
    assert.equal(await sourceBlock.textContent(), original);
    assert.equal(await sourceBlock.isVisible(), true);

    // Recover from errors and verify that user-controlled text stays SVG text.
    await page.evaluate(() => {
      const component = document.querySelector("[notist-function*=grammar]");
      component.setAttribute("notist-source", 'syntax Safe -> `<script>globalThis.grammarInjected = true</script>`;');
    });
    await page.waitForFunction(() => document.querySelector("[notist-function*=grammar]").shadowRoot.querySelector("svg title")?.textContent.trim() === "syntax Safe");
    assert.equal(await page.evaluate(() => globalThis.grammarInjected), undefined);
    assert.equal(await page.locator("[notist-function*=grammar] script").count(), 0);

    // Exercise the Rust base notation through the published WASM API too.
    await page.evaluate(() => {
      document.querySelector("[notist-function*=grammar]").setAttribute("notist-source",
        '@root\nCOMMENT -> `//` ~[`/` `!` LF] ~LF* _line comment_ [^constraint]\nLF -> U+000A\nRAW -> `#`{n:3..=255} <payload> `#`{n}');
    });
    await page.waitForFunction(() => document.querySelector("[notist-function*=grammar]").shadowRoot.querySelector("svg title")?.textContent.trim() === "syntax COMMENT · @root");
    const rustDiagram = await page.locator("[notist-function*=grammar]").first().locator("svg").allTextContents();
    for (const annotation of ["with the exception of", "line comment", "footnote constraint", "U+000A", "total 3..=255 times", "bind repeat count n", "exactly n times"]) {
      assert.ok(rustDiagram.some(text => text.includes(annotation)), `missing ${annotation}`);
    }

    await page.evaluate(() => {
      document.querySelector("[notist-function*=grammar]").setAttribute("notist-source",
        'syntax Heading -> marker:EQ WS body:BODY => heading(level: size(marker))[lower(body)];\nast MD_Heading -> MD.Heading(level, body) => heading(level: level)[lower(body)];');
    });
    await page.waitForFunction(() => document.querySelector("[notist-function*=grammar]").shadowRoot.querySelectorAll("svg").length === 2);
    const construction = await page.locator("[notist-function*=grammar]").first().locator("svg").allTextContents();
    for (const label of ["no input consumption", "construct heading", "lower(body)", "AST node pattern"]) {
      assert.ok(construction.some(text => text.includes(label)), `missing ${label}`);
    }

    await page.evaluate(source => {
      const component = document.querySelector("[notist-function*=grammar]");
      component.setAttribute("notist-source", source);
      component.setAttribute("notist-theme", "rust");
    }, original);
    await page.waitForFunction(() => document.querySelector("[notist-function*=grammar]").shadowRoot.querySelectorAll("svg").length === 4);
    const svg = await page.locator("[notist-function*=grammar]").first().locator("svg").first().evaluate(element => new XMLSerializer().serializeToString(element));
    const output = path.resolve(process.env.NOTIST_GRAMMAR_ARTIFACTS ?? "target/grammar-demo");
    await fs.mkdir(output, { recursive: true });
    await fs.writeFile(path.join(output, "code-call.svg"), svg);
    await page.evaluate(() => window.scrollTo(0, 0));
    await page.screenshot({ path: path.join(output, "preview.png") });
    await page.locator("[notist-function*=grammar]").last().screenshot({ path: path.join(output, "construction.png") });

    if (process.env.NOTIST_TEST_DOCS === "1") {
      for (const [name, file] of [
        ["builtin", "docs/markup/builtin.not"], ["markup", "docs/markup/syntax.not"],
        ["code", "docs/code/syntax.not"], ["notation", "docs/notation.not"],
        ["types", "docs/code/types.not"], ["module", "modules/grammar/README.not"],
      ]) {
        const document = await fs.readFile(path.resolve(file), "utf8");
        const count = (document.match(/#grammar\.diagram\(/g) ?? []).length;
        assert.ok(count > 0, `${name} has visualized rules`);
        await page.goto(`${base}/target/grammar-docs/${name}/`);
        await page.waitForFunction(count => {
          const components = [...document.querySelectorAll("[notist-function*=grammar]")];
          return components.length === count && components.every(component => component.shadowRoot?.querySelector("svg"));
        }, count);
        assert.equal(await page.locator("[notist-function*=grammar] .error").count(), 0, name);
        for (const component of await page.locator("[notist-function*=grammar]").all()) {
          const source = await component.getAttribute("notist-source");
          assert.equal(await component.locator("xpath=..").locator("pre code").first().textContent(), source, name);
          const rules = (source.match(/^(lex|syntax|ast)\s+\w+(?:\([^\n]*?\))?\s*->/gm) ?? []).length;
          const selected = await component.getAttribute("notist-rule");
          assert.equal(await component.locator("svg").count(), selected ? 1 : rules, name);
        }
      }
    }

    // The viewer's existing WASM analyzer and project loader need no grammar-specific changes.
    if (process.env.NOTIST_TEST_WEB === "1") {
      await page.goto(`${base}/web/`);
      await page.waitForSelector("#core .kind");
      await page.fill("#root-url", "../modules/grammar/");
      await page.fill("#config-url", "");
      await page.fill("#path", "README.not");
      const loaded = page.waitForResponse(response => response.url() === `${base}/modules/grammar/README.not`);
      await page.click("#load-project");
      await loaded;
      const source = await fs.readFile(path.resolve("modules/grammar/README.not"), "utf8");
      await page.fill("#path", "README.not");
      await page.locator("#path").dispatchEvent("change");
      await page.fill("#src", source);
      await page.waitForFunction(() => document.querySelector("#core").textContent.includes("/grammar/lib.notc.diagram"));
      const frame = page.frames().find(frame => frame.url() === "about:srcdoc");
      await frame.waitForFunction(() => [...document.querySelectorAll("[notist-function*=grammar]")].length === 6 && [...document.querySelectorAll("[notist-function*=grammar]")].every(component => component.shadowRoot?.querySelector("svg")));
      assert.equal(await frame.locator("[notist-function*=grammar] svg").count(), 17);
    }
    assert.deepEqual(errors, []);
    console.log("Grammar WASM, static resources, links, updates, reconnection, diagnostics and SVG export passed.");
  } finally {
    await browser.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
