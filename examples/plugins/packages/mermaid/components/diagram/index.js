// A browser-ready package may import its own dependencies. Notist executes no JS during analysis.
import mermaid from "https://cdn.jsdelivr.net/npm/mermaid@11.12.0/dist/mermaid.esm.min.mjs";

let nextId = 0;
let queue = Promise.resolve();
export default class Diagram extends HTMLElement {
  static observedAttributes = ["notist-source", "notist-theme"];
  #revision = 0;
  constructor() {
    super();
    this.attachShadow({ mode: "open" });
    this.shadowRoot.innerHTML = "<style>:host { display: block; } svg { max-width: 100%; }</style><div></div>";
  }
  connectedCallback() { this.update(); }
  disconnectedCallback() { this.#revision++; }
  attributeChangedCallback() { if (this.isConnected) this.update(); }
  update() {
    const revision = ++this.#revision;
    const source = this.getAttribute("notist-source") ?? "";
    const requestedTheme = this.getAttribute("notist-theme");
    const theme = ["default", "dark", "forest", "neutral"].includes(requestedTheme) ? requestedTheme : "default";
    const target = this.shadowRoot.querySelector("div");
    target.textContent = source;
    queue = queue.catch(() => {}).then(async () => {
      if (!this.isConnected || revision !== this.#revision) return;
      try {
        mermaid.initialize({ startOnLoad: false, securityLevel: "strict", theme });
        const { svg, bindFunctions } = await mermaid.render(`notist-mermaid-${nextId++}`, source);
        if (!this.isConnected || revision !== this.#revision) return;
        target.innerHTML = svg;
        bindFunctions?.(target);
      } catch (error) {
        if (this.isConnected && revision === this.#revision) target.textContent = `Diagram error: ${error.message}`;
      }
    });
  }
}
