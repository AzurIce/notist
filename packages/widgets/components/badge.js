export default class Badge extends HTMLElement {
  static observedAttributes = ["notist-label", "notist-count"];
  constructor() {
    super();
    this.attachShadow({ mode: "open" });
    this.shadowRoot.innerHTML = "<style>:host { display: inline; padding: .15em .5em; border-radius: .4em; background: #dde8f8; }</style><span></span>";
  }
  connectedCallback() { this.update(); }
  attributeChangedCallback() { this.update(); }
  update() {
    const count = BigInt(this.getAttribute("notist-count") ?? "0");
    this.shadowRoot.querySelector("span").textContent = `${this.getAttribute("notist-label") ?? ""} · ${count}`;
  }
}
