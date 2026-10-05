// Browser module and stylesheet follow KaTeX's documented ESM distribution.
// https://katex.org/docs/browser.html
const stylesheet = "https://cdn.jsdelivr.net/npm/katex@0.19.0/dist/katex.min.css";
let library;
function load() {
  return library ??= import("https://cdn.jsdelivr.net/npm/katex@0.19.0/dist/katex.mjs")
    .then(module => module.default).catch(error => { library = undefined; throw error; });
}

export default class Math extends HTMLElement {
  static observedAttributes = ["notist-text"];
  #revision = 0;
  constructor() {
    super();
    const root = this.attachShadow({ mode: "open" });
    root.innerHTML = '<style>:host { display: inline; } .error { color: #a21; font: inherit; margin-left: .4em; }</style><span class="formula"></span><span class="error" role="status"></span>';
    const link = document.createElement("link");
    link.rel = "stylesheet";
    link.href = stylesheet;
    root.prepend(link);
  }
  connectedCallback() {
    // Register font faces in the document as well as styling the shadow tree.
    if (!this.ownerDocument.querySelector('link[data-notist-katex]')) {
      const link = this.ownerDocument.createElement("link");
      link.rel = "stylesheet";
      link.href = stylesheet;
      link.dataset.notistKatex = "";
      this.ownerDocument.head.append(link);
    }
    this.update();
  }
  disconnectedCallback() { this.#revision++; }
  attributeChangedCallback() { if (this.isConnected) this.update(); }
  async update() {
    const revision = ++this.#revision;
    const source = this.getAttribute("notist-text") ?? "";
    const target = this.shadowRoot.querySelector(".formula");
    const message = this.shadowRoot.querySelector(".error");
    target.textContent = source;
    message.textContent = "";
    try {
      const katex = await load();
      if (!this.isConnected || revision !== this.#revision) return;
      katex.render(source, target, { throwOnError: true, trust: false, output: "htmlAndMathml" });
    } catch (error) {
      if (!this.isConnected || revision !== this.#revision) return;
      target.textContent = source;
      message.textContent = `Math error: ${error.message}`;
    }
  }
}
