// A browser-ready package may import its own dependencies. Notist executes no JS during analysis.
import mermaid from "https://cdn.jsdelivr.net/npm/mermaid@11.12.0/dist/mermaid.esm.min.mjs";

let nextId = 0;
let queue = Promise.resolve();

async function renderWithStableFonts(id, source, target, isCurrent) {
  let fontsChanged = false;
  const onLoading = () => { fontsChanged = true; };
  document.fonts.addEventListener("loading", onLoading);
  let result;
  try {
    result = await mermaid.render(id, source, target);
  } finally {
    document.fonts.removeEventListener("loading", onLoading);
  }
  // Rich labels can request additional faces (e.g. bold text or classDef
  // fonts) only once their CSS exists. Re-layout after those faces settle.
  if (fontsChanged || document.fonts.status === "loading") {
    await document.fonts.ready;
    if (isCurrent()) result = await mermaid.render(id, source, target);
  }
  return result;
}

export default class Diagram extends HTMLElement {
  static observedAttributes = ["notist-source", "notist-theme"];
  #revision = 0;
  #renderedRevision = 0;
  #target;
  constructor() {
    super();
    this.attachShadow({ mode: "open" });
    this.shadowRoot.innerHTML = `<style>
      :host { display: block; min-width: 0; }
      ::slotted(.diagram) { max-width: 100%; overflow-x: auto; }
    </style><slot></slot>`;
  }
  connectedCallback() {
    if (!this.#target) {
      this.#target = document.createElement("div");
      this.#target.className = "diagram";
      this.append(this.#target);
    }
    this.update();
  }
  disconnectedCallback() { this.#revision++; }
  attributeChangedCallback() { if (this.isConnected) this.update(); }
  get renderedSvg() {
    return this.isConnected && this.#renderedRevision === this.#revision
      ? this.#target?.querySelector("svg") ?? null
      : null;
  }
  update() {
    const revision = ++this.#revision;
    const source = this.getAttribute("notist-source") ?? "";
    const requestedTheme = this.getAttribute("notist-theme");
    const theme = ["default", "dark", "forest", "neutral"].includes(requestedTheme) ? requestedTheme : "default";
    const target = this.#target;
    queue = queue.catch(() => {}).then(async () => {
      if (!this.isConnected || revision !== this.#revision) return;
      try {
        mermaid.initialize({ startOnLoad: false, securityLevel: "strict", theme });
        // parse applies source directives; its return value is only their
        // delta. Read the merged defaults, theme and directives from the API.
        // Request its fonts before layout; a failed font uses browser fallback.
        await mermaid.parse(source);
        const config = mermaid.mermaidAPI.getConfig();
        const size = Number.parseFloat(config.themeVariables?.fontSize ?? config.fontSize) || 16;
        const families = new Set([config.fontFamily, config.themeVariables?.fontFamily].map(family => {
          // Mermaid's default fontFamily includes a declaration-ending ";".
          // Parse it as CSS before using it in a FontFaceSet font shorthand.
          const style = document.createElement("span").style;
          style.cssText = `font-family: ${family}`;
          return style.fontFamily;
        }).filter(Boolean));
        await Promise.allSettled([...families].map(family =>
          document.fonts.load(`${size}px ${family}`, source)
        ));
        await document.fonts.ready;
        if (!this.isConnected || revision !== this.#revision) return;
        // Do not clear this container outside the queue: a newer attribute
        // update must not remove an older render's temporary measurement DOM.
        const id = `notist-mermaid-${nextId++}`;
        const { svg, bindFunctions } = await renderWithStableFonts(
          id, source, target, () => this.isConnected && revision === this.#revision
        );
        if (!this.isConnected || revision !== this.#revision) return;
        // Mermaid 11.12 measures inside div#d{id}, with this font-family.
        // Keep that ancestry in the result: inserting SVG directly changes
        // child-selector matches and can clip otherwise valid HTML labels.
        const wrapper = document.createElement("div");
        wrapper.id = `d${id}`;
        wrapper.setAttribute("style", `font-family: ${config.fontFamily}`);
        wrapper.innerHTML = svg;
        target.replaceChildren(wrapper);
        bindFunctions?.(wrapper);
        this.#renderedRevision = revision;
      } catch (error) {
        if (this.isConnected && revision === this.#revision) target.textContent = `Diagram error: ${error.message}`;
      }
    });
  }
}
