import { ShaderRenderer } from "./renderer.js";
import { style } from "./style.js";

function dimension(element, name, fallback) {
  const value = BigInt(element.getAttribute(`notist-${name}`) ?? fallback);
  if (value <= 0n || value > 16384n) throw new Error(`${name} 必须是 1 到 16384 之间的整数。`);
  return Number(value);
}

export default class ShaderCanvas extends HTMLElement {
  static observedAttributes = ["notist-source", "notist-width", "notist-height", "notist-paused"];
  #canvas;
  #shell;
  #stage;
  #status;
  #error;
  #editor;
  #pause;
  #pauseLabel;
  #restart;
  #abort = null;
  #renderer = null;
  #rendererTask = null;
  #resizeObserver;
  #intersectionObserver;
  #revision = 0;
  #queued = false;
  #requestedSource = null;
  #ready = false;
  #paused = false;
  #visible = true;
  #animation = null;
  #time = 0;
  #frame = 0;
  #lastTime = null;
  #lastStatus = -Infinity;
  #mouse = [0, 0, 0, 0];
  #pointer = null;

  constructor() {
    super();
    this.attachShadow({ mode: "open" });
    this.shadowRoot.innerHTML = `<style>${style}</style>
      <div class="shell">
        <div class="stage"><canvas role="img" aria-label="WebGPU shader"></canvas></div>
        <div class="toolbar">
          <div class="controls">
            <button type="button" class="control pause" data-paused="false" aria-label="暂停" title="暂停">
              <svg class="icon pause-icon" viewBox="0 0 16 16" aria-hidden="true"><path d="M5.5 3v10M10.5 3v10"/></svg>
              <svg class="icon play-icon" viewBox="0 0 16 16" aria-hidden="true"><path d="m5 3 8 5-8 5Z"/></svg>
              <span class="label">暂停</span>
            </button>
            <button type="button" class="control restart" aria-label="重置" title="重置">
              <svg class="icon" viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 6A5.5 5.5 0 1 1 3 11M2.5 2.5V6H6"/></svg>
              <span class="label">重置</span>
            </button>
            <button type="button" class="control fullscreen" aria-label="全屏" title="全屏">
              <svg class="icon" viewBox="0 0 16 16" aria-hidden="true"><path d="M6 2.5H2.5V6M10 2.5h3.5V6M13.5 10v3.5H10M6 13.5H2.5V10"/></svg>
              <span class="label">全屏</span>
            </button>
          </div>
          <output aria-label="渲染状态">准备中</output>
        </div>
        <pre class="error" role="alert" hidden></pre>
        <details>
          <summary>
            <svg class="icon chevron" viewBox="0 0 16 16" aria-hidden="true"><path d="m6 4 4 4-4 4"/></svg>
            <span>源码</span><span class="language">WGSL</span>
          </summary>
          <div class="editor">
            <textarea aria-label="WGSL 源码" spellcheck="false" wrap="off" autocapitalize="off" autocomplete="off"></textarea>
            <div class="editor-footer">
              <button type="button" class="apply">
                <svg class="icon" viewBox="0 0 16 16" aria-hidden="true"><path d="m5 3 8 5-8 5Z"/></svg>
                编译并运行
              </button>
              <span class="shortcut" aria-hidden="true">Ctrl / ⌘ + Enter</span>
            </div>
          </div>
        </details>
      </div>`;
    const root = this.shadowRoot;
    this.#canvas = root.querySelector("canvas");
    this.#shell = root.querySelector(".shell");
    this.#stage = root.querySelector(".stage");
    this.#status = root.querySelector("output");
    this.#error = root.querySelector(".error");
    this.#editor = root.querySelector("textarea");
    this.#pause = root.querySelector(".pause");
    this.#pauseLabel = this.#pause.querySelector(".label");
    this.#restart = root.querySelector(".restart");
    this.#pause.addEventListener("click", () => this.setAttribute("notist-paused", String(!this.#paused)));
    this.#restart.addEventListener("click", () => {
      if (this.#ready) { this.#stop(); this.#reset(); this.#schedule(true); }
      else this.#queueUpdate();
    });
    root.querySelector(".fullscreen").addEventListener("click", async () => {
      try {
        if (this.ownerDocument.fullscreenElement === this) await this.ownerDocument.exitFullscreen();
        else await this.requestFullscreen();
      } catch (error) { this.#showError(error); }
    });
    const apply = () => {
      if (this.getAttribute("notist-source") === this.#editor.value) this.#queueUpdate();
      else this.setAttribute("notist-source", this.#editor.value);
    };
    root.querySelector(".apply").addEventListener("click", apply);
    this.#editor.addEventListener("keydown", event => {
      if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) { event.preventDefault(); apply(); }
    });
    this.#canvas.addEventListener("pointerdown", event => {
      if (event.button !== 0 || this.#pointer !== null) return;
      this.#pointer = event.pointerId;
      this.#canvas.setPointerCapture(event.pointerId);
      this.#position(event);
      this.#mouse[2] = Math.max(.0001, this.#mouse[0]);
      this.#mouse[3] = Math.max(.0001, this.#mouse[1]);
      this.#schedule(true);
    });
    this.#canvas.addEventListener("pointermove", event => {
      if (this.#pointer !== null && this.#pointer !== event.pointerId) return;
      this.#position(event);
      this.#schedule(true);
    });
    for (const name of ["pointerup", "pointercancel", "lostpointercapture"]) {
      this.#canvas.addEventListener(name, event => {
        if (this.#pointer !== event.pointerId) return;
        this.#pointer = null;
        this.#mouse[2] = -Math.abs(this.#mouse[2]);
        this.#mouse[3] = -Math.abs(this.#mouse[3]);
        if (this.#canvas.hasPointerCapture(event.pointerId)) this.#canvas.releasePointerCapture(event.pointerId);
        this.#schedule(true);
      });
    }
    this.#resizeObserver = new ResizeObserver(() => this.#schedule(true));
    this.#intersectionObserver = new IntersectionObserver(entries => {
      this.#visible = entries[entries.length - 1].isIntersecting;
      this.#visibility();
    });
  }

  connectedCallback() {
    this.#abort = new AbortController();
    this.#visible = true;
    this.#resizeObserver.observe(this.#stage);
    this.#intersectionObserver.observe(this);
    this.ownerDocument.addEventListener("visibilitychange", this.#visibility);
    this.#queueUpdate();
  }

  disconnectedCallback() {
    this.#revision++;
    this.#abort.abort();
    this.#stop();
    this.#resizeObserver.disconnect();
    this.#intersectionObserver.disconnect();
    this.ownerDocument.removeEventListener("visibilitychange", this.#visibility);
    this.#renderer?.dispose();
    this.#renderer = null;
    this.#rendererTask = null;
    this.#requestedSource = null;
    this.#ready = false;
    this.#pointer = null;
  }

  attributeChangedCallback() { if (this.isConnected) this.#queueUpdate(); }

  #queueUpdate() {
    if (this.#queued) return;
    this.#queued = true;
    queueMicrotask(() => {
      this.#queued = false;
      if (this.isConnected) this.#update();
    });
  }

  #update() {
    try {
      const width = dimension(this, "width", 960);
      const height = dimension(this, "height", 540);
      const source = this.getAttribute("notist-source") ?? "";
      this.#shell.style.maxWidth = `${width}px`;
      this.#stage.style.aspectRatio = `${width} / ${height}`;
      this.#paused = this.getAttribute("notist-paused") === "true";
      const pauseLabel = this.#paused ? "播放" : "暂停";
      this.#pauseLabel.textContent = pauseLabel;
      this.#pause.dataset.paused = String(this.#paused);
      this.#pause.setAttribute("aria-label", pauseLabel);
      this.#pause.title = pauseLabel;
      this.#stop();
      if (source !== this.#requestedSource || !this.#rendererTask || this.dataset.state === "error") {
        this.#requestedSource = source;
        this.#editor.value = source;
        this.#compile(source);
      } else this.#schedule(true);
    } catch (error) {
      this.#revision++;
      this.#ready = false;
      this.#stop();
      this.#showError(error);
    }
  }

  async #compile(source) {
    const revision = ++this.#revision;
    this.#ready = false;
    this.dataset.state = "loading";
    this.#status.textContent = "编译中";
    this.#error.hidden = true;
    this.#error.textContent = "";
    this.#pause.disabled = true;
    this.#restart.disabled = true;
    try {
      if (!this.#rendererTask) {
        const signal = this.#abort.signal;
        this.#rendererTask = ShaderRenderer.create(this.#canvas, error => {
          if (signal.aborted) return;
          this.#revision++;
          this.#ready = false;
          this.#stop();
          this.#renderer?.dispose();
          this.#renderer = null;
          this.#rendererTask = null;
          this.#showError(error);
        }, signal).then(renderer => {
          if (signal.aborted) { renderer.dispose(); signal.throwIfAborted(); }
          this.#renderer = renderer;
          return renderer;
        });
      }
      const renderer = await this.#rendererTask;
      if (!this.isConnected || revision !== this.#revision) return;
      const compiled = await renderer.compile(source);
      if (!compiled || !this.isConnected || revision !== this.#revision) return;
      this.#ready = true;
      this.dataset.state = "ready";
      this.#status.textContent = "就绪";
      this.#pause.disabled = false;
      this.#restart.disabled = false;
      this.#reset();
      this.#schedule(true);
    } catch (error) {
      if (this.isConnected && revision === this.#revision) {
        if (!this.#renderer) this.#rendererTask = null;
        this.#showError(error);
      }
    }
  }

  #showError(error) {
    this.dataset.state = "error";
    this.#status.textContent = "发生错误";
    this.#error.textContent = error.message ?? String(error);
    this.#error.hidden = false;
    this.#pause.disabled = !this.#ready;
    this.#restart.disabled = false;
  }

  #reset() {
    this.#time = 0;
    this.#frame = 0;
    this.#lastTime = null;
    this.#lastStatus = -Infinity;
    this.#mouse = [0, 0, 0, 0];
  }

  #stop() {
    if (this.#animation !== null) cancelAnimationFrame(this.#animation);
    this.#animation = null;
    this.#lastTime = null;
    this.#lastStatus = -Infinity;
  }

  #visibility = () => {
    this.#stop();
    this.#schedule(true);
  };

  #schedule(once = false) {
    if (!this.#ready || !this.isConnected || !this.#visible || this.ownerDocument.hidden
      || this.#animation !== null || (this.#paused && !once)) return;
    this.#animation = requestAnimationFrame(now => {
      this.#animation = null;
      try {
        this.#renderer.resize(this.ownerDocument.defaultView.devicePixelRatio);
        const delta = !this.#paused && this.#lastTime !== null ? (now - this.#lastTime) / 1000 : 0;
        this.#time += delta;
        this.#lastTime = this.#paused ? null : now;
        this.#renderer.draw({ time: this.#time, delta, frame: this.#frame, mouse: this.#mouse });
        if (now - this.#lastStatus >= 250) {
          this.#status.textContent = `${this.#canvas.width} × ${this.#canvas.height} · ${this.#time.toFixed(2)} s · 帧 ${this.#frame}`;
          this.#lastStatus = now;
        }
        this.#frame++;
        this.#schedule();
      } catch (error) {
        this.#ready = false;
        this.#stop();
        this.#showError(error);
      }
    });
  }

  #position(event) {
    const rect = this.#canvas.getBoundingClientRect();
    if (!rect.width || !rect.height) return;
    this.#mouse[0] = Math.max(0, Math.min(this.#canvas.width, (event.clientX - rect.left) * this.#canvas.width / rect.width));
    this.#mouse[1] = Math.max(0, Math.min(this.#canvas.height, (rect.bottom - event.clientY) * this.#canvas.height / rect.height));
  }
}
