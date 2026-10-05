import init, { render_grammar } from "./wasm/grammar.js";

// Each component uses the same lazily initialized Rust/WASM engine.
let engine;
function initialize() {
  engine ??= init().catch(error => { engine = undefined; throw error; });
  return engine;
}

export default class GrammarDiagram extends HTMLElement {
  static observedAttributes = ["notist-source", "notist-rule", "notist-theme"];
  #revision = 0;

  constructor() {
    super();
    this.attachShadow({ mode: "open" });
    this.shadowRoot.innerHTML = `<style>
      :host { display: block; margin-block: 1em; }
      .diagrams { overflow-x: auto; }
      svg { display: block; height: auto; margin-block: .5em; }
      .source { background: #f6f8fa; color: #24292f; padding: 1em; border-radius: 6px; }
      pre { white-space: pre-wrap; overflow-wrap: anywhere; }
      code { font: inherit; }
      .error { color: #b42318; }
    </style><pre class="source"><code></code></pre><div class="diagrams" aria-live="polite"></div>`;
    this.shadowRoot.addEventListener("click", event => {
      const link = event.target.closest?.("a");
      const href = link?.getAttribute("href") ?? link?.getAttribute("xlink:href");
      if (!href?.startsWith("#")) return;
      const target = this.shadowRoot.getElementById(href.slice(1));
      if (target) { event.preventDefault(); target.scrollIntoView({ block: "nearest" }); }
    });
  }

  connectedCallback() { this.update(); }
  disconnectedCallback() { this.#revision++; }
  attributeChangedCallback() { if (this.isConnected) this.update(); }

  async update() {
    const revision = ++this.#revision;
    const source = this.getAttribute("notist-source") ?? "";
    const rule = this.getAttribute("notist-rule") ?? "";
    const theme = this.getAttribute("notist-theme") ?? "light";
    const target = this.shadowRoot.querySelector(".diagrams");
    // A single source drives both the persistent code block and the diagrams.
    this.shadowRoot.querySelector(".source code").textContent = source;
    target.replaceChildren();
    try {
      await initialize();
      if (!this.isConnected || revision !== this.#revision) return;
      // Rust emits only SVG; source strings are escaped by the SVG renderer.
      target.innerHTML = render_grammar(source, rule, theme);
      this.dispatchEvent(new CustomEvent("grammar-rendered"));
    } catch (error) {
      if (!this.isConnected || revision !== this.#revision) return;
      const message = document.createElement("pre");
      message.className = "error";
      message.setAttribute("role", "alert");
      message.textContent = `Grammar error: ${error.message ?? error}`;
      target.replaceChildren(message);
      this.dispatchEvent(new CustomEvent("grammar-error", { detail: String(error) }));
    }
  }
}
