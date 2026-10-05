import { style } from "./style.js";

export default class Panel extends HTMLElement {
  static observedAttributes = ["notist-title", "notist-expanded"];
  constructor() {
    super();
    this.attachShadow({ mode: "open" });
    this.shadowRoot.innerHTML = `<style>${style}</style><details><summary></summary><slot></slot></details>`;
  }
  connectedCallback() { this.update(); }
  attributeChangedCallback() { this.update(); }
  update() {
    const details = this.shadowRoot.querySelector("details");
    details.open = this.getAttribute("notist-expanded") === "true";
    this.shadowRoot.querySelector("summary").textContent = this.getAttribute("notist-title") ?? "";
  }
}
