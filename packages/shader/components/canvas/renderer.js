// Append the host declarations so user source keeps its line numbers and may
// start with WGSL enable/requires/diagnostic directives.
const hostSource = `
struct NotistShaderUniforms {
  iResolution: vec3f,
  iTime: f32,
  iTimeDelta: f32,
  iFrame: u32,
  iFrameRate: f32,
  iMouse: vec4f,
  iDate: vec4f,
}
@group(0) @binding(0) var<uniform> uniforms: NotistShaderUniforms;

@vertex fn notistShaderVertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
  let positions = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
  return vec4f(positions[index], 0.0, 1.0);
}

@fragment fn notistShaderFragment(@builtin(position) position: vec4f) -> @location(0) vec4f {
  return mainImage(vec2f(position.x, uniforms.iResolution.y - position.y));
}
`;

export class ShaderRenderer {
  #canvas;
  #device;
  #context;
  #format;
  #buffer;
  #bindGroup;
  #layout;
  #pipeline = null;
  #revision = 0;
  #disposed = false;
  #onError;
  #bytes = new ArrayBuffer(64);
  #floats = new Float32Array(this.#bytes);
  #integers = new Uint32Array(this.#bytes);

  static async create(canvas, onError, signal) {
    if (!globalThis.isSecureContext) throw new Error("WebGPU 需要 HTTPS 或 localhost 安全上下文。");
    if (!navigator.gpu) throw new Error("此浏览器不支持 WebGPU，请使用支持 WebGPU 的浏览器。");
    signal.throwIfAborted();
    const adapter = await navigator.gpu.requestAdapter();
    signal.throwIfAborted();
    if (!adapter) throw new Error("未找到可用的 WebGPU 适配器。");
    const device = await adapter.requestDevice({ label: "Notist shader" });
    try {
      signal.throwIfAborted();
      return new ShaderRenderer(canvas, device, onError);
    } catch (error) {
      device.destroy();
      throw error;
    }
  }

  constructor(canvas, device, onError) {
    this.#canvas = canvas;
    this.#device = device;
    this.#onError = onError;
    this.#context = canvas.getContext("webgpu");
    if (!this.#context) throw new Error("无法创建 WebGPU canvas 上下文。");
    this.#format = navigator.gpu.getPreferredCanvasFormat();
    device.addEventListener("uncapturederror", this.#gpuError);
    device.lost.then(info => {
      if (!this.#disposed) onError(new Error(`WebGPU 设备丢失：${info.message || info.reason}`));
    });
    this.#buffer = device.createBuffer({
      label: "ShaderToy uniforms",
      size: this.#bytes.byteLength,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    const bindings = device.createBindGroupLayout({
      entries: [{ binding: 0, visibility: GPUShaderStage.FRAGMENT, buffer: { type: "uniform", minBindingSize: 64 } }],
    });
    this.#layout = device.createPipelineLayout({ bindGroupLayouts: [bindings] });
    this.#bindGroup = device.createBindGroup({
      layout: bindings,
      entries: [{ binding: 0, resource: { buffer: this.#buffer } }],
    });
    this.#context.configure({ device, format: this.#format, alphaMode: "opaque" });
  }

  #gpuError = event => {
    event.preventDefault();
    if (!this.#disposed) this.#onError(new Error(`WebGPU 错误：${event.error.message}`));
  };

  async compile(source) {
    const revision = ++this.#revision;
    // Pop the scope before awaiting: overlapping compilations must not share
    // an async error-scope stack. Compilation failures are displayed in the UI.
    this.#device.pushErrorScope("validation");
    const module = this.#device.createShaderModule({ label: "mainImage", code: source + "\n" + hostSource });
    const validation = this.#device.popErrorScope();
    const [info, error] = await Promise.all([module.getCompilationInfo(), validation]);
    if (this.#disposed || revision !== this.#revision) return false;
    const messages = info.messages.filter(message => message.type === "error");
    if (messages.length) {
      const lines = source.split("\n").length;
      throw new Error(messages.map(message => {
        const location = message.lineNum > lines ? "入口 / uniform" : `${message.lineNum}:${message.linePos}`;
        return `WGSL ${location}: ${message.message}`;
      }).join("\n"));
    }
    if (error) throw new Error(error.message);
    const pipeline = await this.#device.createRenderPipelineAsync({
      label: "Notist mainImage",
      layout: this.#layout,
      vertex: { module, entryPoint: "notistShaderVertex" },
      fragment: { module, entryPoint: "notistShaderFragment", targets: [{ format: this.#format }] },
      primitive: { topology: "triangle-list" },
    });
    if (this.#disposed || revision !== this.#revision) return false;
    this.#pipeline = pipeline;
    return true;
  }

  resize(pixelRatio) {
    const rect = this.#canvas.getBoundingClientRect();
    const ratio = Math.min(2, pixelRatio || 1);
    const width = Math.max(1, Math.round(rect.width * ratio));
    const height = Math.max(1, Math.round(rect.height * ratio));
    const scale = Math.min(1, this.#device.limits.maxTextureDimension2D / Math.max(width, height));
    const targetWidth = Math.max(1, Math.floor(width * scale));
    const targetHeight = Math.max(1, Math.floor(height * scale));
    if (this.#canvas.width !== targetWidth) this.#canvas.width = targetWidth;
    if (this.#canvas.height !== targetHeight) this.#canvas.height = targetHeight;
  }

  draw({ time, delta, frame, mouse, date = new Date() }) {
    if (this.#disposed || !this.#pipeline) return;
    // WGSL offsets: resolution/time 0..15, delta/frame/rate 16..27,
    // padding 28..31, mouse 32..47, date 48..63.
    this.#floats.set([this.#canvas.width, this.#canvas.height, 1, time, delta]);
    this.#integers[5] = frame;
    this.#floats[6] = delta > 0 ? 1 / delta : 0;
    this.#floats.set(mouse, 8);
    this.#floats.set([
      date.getFullYear(), date.getMonth() + 1, date.getDate(),
      date.getHours() * 3600 + date.getMinutes() * 60 + date.getSeconds() + date.getMilliseconds() / 1000,
    ], 12);
    this.#device.queue.writeBuffer(this.#buffer, 0, this.#bytes);
    const encoder = this.#device.createCommandEncoder();
    const pass = encoder.beginRenderPass({
      colorAttachments: [{
        view: this.#context.getCurrentTexture().createView(),
        clearValue: { r: 0, g: 0, b: 0, a: 1 },
        loadOp: "clear", storeOp: "store",
      }],
    });
    pass.setPipeline(this.#pipeline);
    pass.setBindGroup(0, this.#bindGroup);
    pass.draw(3);
    pass.end();
    this.#device.queue.submit([encoder.finish()]);
  }

  dispose() {
    if (this.#disposed) return;
    this.#disposed = true;
    this.#revision++;
    this.#device.removeEventListener("uncapturederror", this.#gpuError);
    this.#context.unconfigure();
    this.#buffer.destroy();
    this.#device.destroy();
    this.#pipeline = null;
  }
}
