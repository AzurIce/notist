import init, { render_grammar } from "./wasm/grammar.js";

// Each component uses the same lazily initialized Rust/WASM engine.
let engine;
function initialize() {
  engine ??= init().catch(error => { engine = undefined; throw error; });
  return engine;
}

export default class GrammarGraph extends HTMLElement {
  static observedAttributes = ["notist-source", "notist-rule", "notist-theme"];
  #revision = 0;
  #media;
  #themeObserver;
  #refresh = () => { if (this.isConnected) this.update(); };

  constructor() {
    super();
    this.attachShadow({ mode: "open" });
    this.shadowRoot.innerHTML = `<style>
      :host { display: block; }
      .diagrams { overflow-x: auto; }
      svg { display: block; height: auto; margin-block: .5em; }
      pre.error { white-space: pre-wrap; overflow-wrap: anywhere; }
      .error { color: #b42318; }
    </style><div class="diagrams" aria-live="polite"></div>`;
    this.shadowRoot.addEventListener("click", event => {
      const link = event.target.closest?.("a");
      const href = link?.getAttribute("href") ?? link?.getAttribute("xlink:href");
      if (!href?.startsWith("#")) return;
      const target = this.shadowRoot.getElementById(href.slice(1));
      if (target) { event.preventDefault(); target.scrollIntoView({ block: "nearest" }); }
    });
  }

  #theme() {
    const requested = this.getAttribute("notist-theme");
    if (requested && requested !== "auto") return requested;
    const schemes = this.ownerDocument.defaultView.getComputedStyle(this).colorScheme.split(/\s+/);
    const dark = schemes.includes("dark"), light = schemes.includes("light");
    return (dark !== light ? dark : this.#media?.matches) ? "dark" : "light";
  }

  connectedCallback() {
    this.#media = this.ownerDocument.defaultView.matchMedia("(prefers-color-scheme: dark)");
    this.#media.addEventListener("change", this.#refresh);
    this.#themeObserver = new MutationObserver(this.#refresh);
    // Follow inherited theme changes like other diagram components.
    for (let ancestor = this; ancestor; ancestor = ancestor.parentElement ?? ancestor.getRootNode().host) {
      this.#themeObserver.observe(ancestor, { attributes: true, attributeFilter: ["data-theme", "class", "style"] });
    }
    this.update();
  }
  disconnectedCallback() {
    this.#revision++;
    this.#media?.removeEventListener("change", this.#refresh);
    this.#themeObserver?.disconnect();
  }
  attributeChangedCallback() { if (this.isConnected && this.#media) this.update(); }

  async update() {
    const revision = ++this.#revision;
    const source = this.getAttribute("notist-source") ?? "";
    const rule = this.getAttribute("notist-rule") ?? "";
    const theme = this.#theme();
    const target = this.shadowRoot.querySelector(".diagrams");
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
