// Export packages/shader/README.not to target/shader-demo and serve the repo.
// Playwright must be on NODE_PATH; web/pkg is needed for the Worker preview.
const assert = require("node:assert/strict");
const { chromium } = require("playwright");

const red = "fn mainImage(p: vec2f) -> vec4f { return vec4f(1.0, 0.0, 0.0, 1.0); }";
const blue = "fn mainImage(p: vec2f) -> vec4f { return vec4f(0.0, 0.0, 1.0, 1.0); }";

// Observe real GPU calls without replacing compilation, buffers or rendering.
function instrument() {
  const state = window.shaderTest = { devices: [], holdDevice: false, holdPipeline: false };
  const requestDevice = GPUAdapter.prototype.requestDevice;
  GPUAdapter.prototype.requestDevice = async function (...args) {
    const hold = state.holdDevice;
    state.holdDevice = false;
    const device = await requestDevice.apply(this, args);
    state.devices.push({ device, queue: device.queue, submissions: 0, writes: 0, destroyed: false });
    if (hold) await new Promise(resolve => { state.releaseDevice = resolve; });
    return device;
  };
  const configure = GPUCanvasContext.prototype.configure;
  GPUCanvasContext.prototype.configure = function (options) {
    const record = this.canvas.shaderRecord = state.devices.find(record => record.device === options.device);
    record.format = options.format;
    return configure.call(this, {
      ...options, usage: (options.usage ?? GPUTextureUsage.RENDER_ATTACHMENT) | GPUTextureUsage.COPY_SRC,
    });
  };
  const getCurrentTexture = GPUCanvasContext.prototype.getCurrentTexture;
  GPUCanvasContext.prototype.getCurrentTexture = function (...args) {
    const texture = getCurrentTexture.apply(this, args);
    this.canvas.shaderRecord.texture = texture;
    return texture;
  };
  const submit = GPUQueue.prototype.submit;
  GPUQueue.prototype.submit = function (...args) {
    const record = state.devices.find(record => record.queue === this);
    if (record) record.submissions++;
    const result = submit.apply(this, args);
    if (record?.texture) {
      // Copy pixels before canvas presentation expires the current texture.
      // This checks GPU output independently of the platform compositor.
      const { device, texture, format } = record;
      const buffer = device.createBuffer({ size: 512, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ });
      const encoder = device.createCommandEncoder();
      const points = [[Math.min(1, texture.width - 1), Math.min(1, texture.height - 1)],
        [Math.max(0, texture.width - 2), Math.max(0, texture.height - 2)]];
      points.forEach(([x, y], index) => encoder.copyTextureToBuffer(
        { texture, origin: { x, y } }, { buffer, offset: index * 256, bytesPerRow: 256 }, [1, 1]));
      submit.call(this, [encoder.finish()]);
      record.readback = buffer.mapAsync(GPUMapMode.READ).then(() => {
        const bytes = new Uint8Array(buffer.getMappedRange());
        const colors = [0, 256].map(offset => {
          const color = Array.from(bytes.slice(offset, offset + 4));
          return format.startsWith("bgra") ? [color[2], color[1], color[0], color[3]] : color;
        });
        buffer.unmap();
        buffer.destroy();
        return colors;
      }).catch(error => { buffer.destroy(); return { error: error.message }; });
    }
    return result;
  };
  const writeBuffer = GPUQueue.prototype.writeBuffer;
  GPUQueue.prototype.writeBuffer = function (...args) {
    const record = state.devices.find(record => record.queue === this);
    if (record) {
      const floats = new Float32Array(args[2]);
      record.writes++;
      record.uniforms = {
        resolution: Array.from(floats.slice(0, 3)), time: floats[3], delta: floats[4],
        frame: new Uint32Array(args[2])[5], rate: floats[6],
        mouse: Array.from(floats.slice(8, 12)), date: Array.from(floats.slice(12, 16)),
      };
    }
    return writeBuffer.apply(this, args);
  };
  const destroy = GPUDevice.prototype.destroy;
  GPUDevice.prototype.destroy = function (...args) {
    const record = state.devices.find(record => record.device === this);
    if (record) record.destroyed = true;
    return destroy.apply(this, args);
  };
  const createPipeline = GPUDevice.prototype.createRenderPipelineAsync;
  GPUDevice.prototype.createRenderPipelineAsync = async function (...args) {
    const hold = state.holdPipeline;
    state.holdPipeline = false;
    const pipeline = await createPipeline.apply(this, args);
    if (hold) await new Promise(resolve => { state.releasePipeline = resolve; });
    return pipeline;
  };
}

async function frames(page, count = 8) {
  await page.evaluate(count => new Promise(resolve => {
    const next = () => --count <= 0 ? resolve() : requestAnimationFrame(next);
    requestAnimationFrame(next);
  }), count);
}

async function ready(node) {
  await node.page().waitForFunction(() => [...document.querySelectorAll("shader-canvas")]
    .every(node => node.dataset.state === "ready"));
  await node.scrollIntoViewIfNeeded();
  await frames(node.page(), 3);
  assert.equal(await node.locator(".error").textContent(), "");
}

async function record(node) {
  return node.evaluate(node => {
    const { submissions, writes, destroyed, uniforms } = node.shadowRoot.querySelector("canvas").shaderRecord;
    return { submissions, writes, destroyed, uniforms };
  });
}

async function pixel(node, x = 1, y = 1) {
  return node.evaluate(async (node, [x, y]) => {
    const colors = await node.shadowRoot.querySelector("canvas").shaderRecord.readback;
    if (colors.error) throw new Error(colors.error);
    return colors[x === 1 && y === 1 ? 0 : 1];
  }, [x, y]);
}

(async () => {
  const browser = await chromium.launch({
    headless: process.env.NOTIST_HEADLESS !== "false",
    args: ["--enable-unsafe-webgpu", ...JSON.parse(process.env.NOTIST_WEBGPU_ARGS ?? "[]")],
    ...(process.env.NOTIST_CHROMIUM ? { executablePath: process.env.NOTIST_CHROMIUM } : {}),
  });
  try {
    const base = process.env.NOTIST_TEST_URL ?? "http://127.0.0.1:8000";
    const page = await browser.newPage({ viewport: { width: 1200, height: 900 }, deviceScaleFactor: 2 });
    page.setDefaultTimeout(30000);
    await page.addInitScript(instrument);
    const errors = [];
    const requests = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("request", request => requests.push(request.url()));
    await page.goto(`${base}/target/shader-demo/`);
    assert.equal(await page.evaluate(async () => !!(await navigator.gpu?.requestAdapter())), true,
      "A real WebGPU adapter is required; set NOTIST_WEBGPU_ARGS for your test GPU.");
    assert.equal(await page.locator("shader-canvas").count(), 2);
    const shader = page.locator("shader-canvas").first();
    await ready(shader);
    assert.ok((await record(shader)).submissions > 0, "the demo submits real GPU work");
    await page.locator("shader-canvas").nth(1).scrollIntoViewIfNeeded();
    await frames(page);
    await page.screenshot({ path: "target/shader-demo/preview.png", fullPage: true });
    assert.ok(requests.every(url => new URL(url).origin === new URL(base).origin), "no CDN dependencies");

    await shader.scrollIntoViewIfNeeded();
    await shader.evaluate((node, source) => {
      node.setAttribute("notist-width", "256");
      node.setAttribute("notist-height", "128");
      node.setAttribute("notist-paused", "true");
      node.setAttribute("notist-source", source);
    }, red);
    await ready(shader);
    assert.deepEqual(await pixel(shader), [255, 0, 0, 255]);
    let current = await record(shader);
    assert.deepEqual(current.uniforms.resolution, [512, 256, 1]);
    assert.equal(current.uniforms.time, 0);
    assert.equal(current.uniforms.delta, 0);
    assert.equal(current.uniforms.rate, 0);
    const date = await page.evaluate(() => { const date = new Date(); return [date.getFullYear(), date.getMonth() + 1, date.getDate()]; });
    assert.deepEqual(current.uniforms.date.slice(0, 3), date);
    assert.ok(current.uniforms.date[3] >= 0 && current.uniforms.date[3] < 86400);
    await frames(page);
    assert.equal((await record(shader)).submissions, current.submissions, "pause stops the frame loop");

    await shader.locator(".pause").click();
    await frames(page, 12);
    current = await record(shader);
    assert.ok(current.uniforms.time > 0 && current.uniforms.delta > 0 && current.uniforms.frame > 0);
    assert.ok(Math.abs(current.uniforms.rate * current.uniforms.delta - 1) < .001);
    await shader.locator(".pause").click();
    await frames(page);
    current = await record(shader);
    await frames(page);
    assert.equal((await record(shader)).submissions, current.submissions);
    await shader.locator(".restart").click();
    await frames(page);
    current = await record(shader);
    assert.equal(current.uniforms.time, 0);
    assert.equal(current.uniforms.frame, 0, "restart draws frame zero");

    await shader.evaluate(node => node.setAttribute("notist-source", `
      fn mainImage(p: vec2f) -> vec4f {
        return vec4f(p / uniforms.iResolution.xy, 0.0, 1.0);
      }`));
    await ready(shader);
    const top = await pixel(shader, 1, 1);
    const bottom = await pixel(shader, 510, 254);
    assert.ok(top[0] < 5 && top[1] > 250 && bottom[0] > 250 && bottom[1] < 5,
      "pixel coordinates have a lower-left origin");

    const canvas = shader.locator("canvas");
    const box = await canvas.boundingBox();
    await page.mouse.move(box.x + box.width / 4, box.y + box.height / 4);
    await page.mouse.down();
    await frames(page);
    current = await record(shader);
    assert.deepEqual(current.uniforms.mouse, [128, 192, 128, 192]);
    await page.mouse.move(box.x + box.width + 20, box.y + box.height + 20);
    await frames(page);
    current = await record(shader);
    assert.deepEqual(current.uniforms.mouse, [512, 0, 128, 192]);
    await page.mouse.up();
    await frames(page);
    assert.deepEqual((await record(shader)).uniforms.mouse, [512, 0, -128, -192]);

    const deviceCount = await page.evaluate(() => shaderTest.devices.length);
    await shader.evaluate(node => node.style.width = "128px");
    await frames(page);
    assert.deepEqual((await record(shader)).uniforms.resolution, [256, 128, 1]);
    assert.equal(await page.evaluate(() => shaderTest.devices.length), deviceCount, "resize reuses the GPU device");
    await shader.evaluate(node => node.style.width = "");
    await frames(page);

    await shader.locator("summary").click();
    await shader.locator("textarea").fill(blue);
    await shader.locator(".apply").click();
    await ready(shader);
    assert.equal(await shader.getAttribute("notist-source"), blue);
    assert.deepEqual(await pixel(shader), [0, 0, 255, 255]);
    await shader.locator("textarea").fill("fn mainImage(p: vec2f) -> vec4f {\n return missing;\n}");
    await shader.locator("textarea").press("Control+Enter");
    await page.waitForFunction(() => document.querySelector("shader-canvas").dataset.state === "error");
    assert.match(await shader.locator(".error").textContent(), /WGSL 2:\d+:/);
    assert.deepEqual(await pixel(shader), [0, 0, 255, 255], "compile errors preserve the previous image");
    await shader.locator("textarea").fill(red);
    await shader.locator(".apply").click();
    await ready(shader);
    await shader.locator("summary").click();

    await shader.evaluate((node, source) => node.setAttribute("notist-source",
      "diagnostic(off, derivative_uniformity);\n" + source), red);
    await ready(shader);
    assert.deepEqual(await pixel(shader), [255, 0, 0, 255], "user directives precede host declarations");
    await shader.locator(".fullscreen").click();
    await page.waitForFunction(() => document.fullscreenElement === document.querySelector("shader-canvas"));
    await frames(page, 3);
    assert.ok((await record(shader)).uniforms.resolution[0] > 512, "fullscreen resizes the render target");
    await shader.locator(".fullscreen").click();
    await page.waitForFunction(() => !document.fullscreenElement);
    await frames(page, 3);

    // An older valid compilation finishing late must not replace a newer one.
    await page.evaluate(() => { shaderTest.holdPipeline = true; });
    await shader.evaluate((node, source) => node.setAttribute("notist-source", source), blue);
    await page.waitForFunction(() => !!shaderTest.releasePipeline);
    await shader.evaluate((node, source) => node.setAttribute("notist-source", source), red);
    await ready(shader);
    await page.evaluate(() => shaderTest.releasePipeline());
    await frames(page);
    assert.deepEqual(await pixel(shader), [255, 0, 0, 255]);

    await shader.evaluate(node => node.setAttribute("notist-width", "9223372036854775807"));
    await page.waitForFunction(() => document.querySelector("shader-canvas").dataset.state === "error");
    assert.match(await shader.locator(".error").textContent(), /width.*1.*16384/);
    await shader.evaluate(node => node.setAttribute("notist-width", "256"));
    await ready(shader);

    await shader.evaluate(node => node.setAttribute("notist-paused", "false"));
    await frames(page);
    await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
    await frames(page);
    current = await record(shader);
    await frames(page);
    assert.equal((await record(shader)).submissions, current.submissions, "offscreen animation stops");
    await shader.scrollIntoViewIfNeeded();
    await frames(page, 4);
    assert.ok((await record(shader)).submissions > current.submissions, "onscreen animation resumes");

    await shader.evaluate(node => {
      const canvas = node.shadowRoot.querySelector("canvas");
      window.shaderBeforeDisconnect = canvas.shaderRecord;
      window.shaderParent = node.parentNode;
      window.shaderNext = node.nextSibling;
      window.shaderDetached = node;
      node.remove();
    });
    const detached = await page.evaluate(() => ({ destroyed: shaderBeforeDisconnect.destroyed, submissions: shaderBeforeDisconnect.submissions }));
    assert.equal(detached.destroyed, true);
    await frames(page);
    assert.equal(await page.evaluate(() => shaderBeforeDisconnect.submissions), detached.submissions);
    await page.evaluate(() => shaderParent.insertBefore(shaderDetached, shaderNext));
    await ready(shader);
    assert.notEqual(await shader.evaluate(node => node.shadowRoot.querySelector("canvas").shaderRecord === shaderBeforeDisconnect), true);

    // Device loss is reported and retry creates a fresh device.
    await shader.evaluate(node => node.shadowRoot.querySelector("canvas").shaderRecord.device.destroy());
    await page.waitForFunction(() => document.querySelector("shader-canvas").dataset.state === "error");
    assert.match(await shader.locator(".error").textContent(), /设备丢失/);
    await shader.locator(".restart").click();
    await ready(shader);

    // A device obtained after removal must be destroyed before configuring the
    // reconnected canvas, even if a newer device has already rendered there.
    await shader.evaluate(node => {
      node.remove();
      shaderTest.holdDevice = true;
      shaderParent.insertBefore(node, shaderNext);
    });
    await page.waitForFunction(() => !!shaderTest.releaseDevice);
    await shader.evaluate(node => {
      window.shaderHeldDevice = shaderTest.devices.at(-1);
      node.remove();
      shaderParent.insertBefore(node, shaderNext);
    });
    await ready(shader);
    await page.evaluate(() => shaderTest.releaseDevice());
    await frames(page);
    assert.equal(await page.evaluate(() => shaderHeldDevice.destroyed), true);
    assert.deepEqual(await pixel(shader), [255, 0, 0, 255]);

    // The existing Worker host discovers and loads the same package entry.
    await page.goto(`${base}/web/`);
    await page.waitForSelector("#core .kind");
    await page.fill("#config-url", "../packages/shader/Notist.toml");
    await page.fill("#path", "README.not");
    await page.click("#load-project");
    await page.waitForFunction(() => {
      const nodes = [...(document.querySelector("#preview").contentDocument?.querySelectorAll("shader-canvas") ?? [])];
      return nodes.length === 2 && nodes.every(node => node.dataset.state === "ready");
    });
    assert.equal(await page.locator("#diags").textContent(), "✓ 无诊断");
    assert.match(await page.locator("#core").textContent(), /shader::canvas/);
    await page.fill("#src", `#shader::canvas("${red}", paused: true)`);
    await page.waitForFunction(() => {
      const nodes = document.querySelector("#preview").contentDocument?.querySelectorAll("shader-canvas");
      return nodes?.length === 1 && nodes[0].dataset.state === "ready";
    });

    for (const unavailable of ["api", "adapter"]) {
      const fallback = await browser.newPage();
      fallback.on("pageerror", error => errors.push(error.message));
      await fallback.addInitScript(unavailable => {
        if (unavailable === "api") Object.defineProperty(navigator, "gpu", { value: undefined });
        else GPU.prototype.requestAdapter = async () => null;
      }, unavailable);
      await fallback.goto(`${base}/target/shader-demo/`);
      await fallback.waitForFunction(() => [...document.querySelectorAll("shader-canvas")]
        .every(node => node.dataset.state === "error"));
      assert.match(await fallback.locator("shader-canvas .error").first().textContent(),
        unavailable === "api" ? /不支持 WebGPU/ : /适配器/);
      await fallback.close();
    }
    assert.deepEqual(errors, []);
    console.log("WebGPU pixels, uniforms, controls, mouse, resizing, compilation races, lifecycle, device loss, errors and Worker preview passed.");
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
