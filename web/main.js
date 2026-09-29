import init, { analyze } from "./pkg/notist.js";

const srcEl = document.querySelector("#src");
const tokensEl = document.querySelector("#tokens");
const treeEl = document.querySelector("#tree");
const astEl = document.querySelector("#ast");
const coreEl = document.querySelector("#core");
const showwsEl = document.querySelector("#showws");
const corpusEl = document.querySelector("#corpus");

const SAMPLE = "= Title\n\nfirst line\nsecond\tline\n   \n== Sub\n\n=x\n";

function visible(text) {
  return text
    .replace(/ /g, "·")
    .replace(/\t/g, "→\t")
    .replace(/\r\n|\r|\n/g, (m) => "⏎" + m);
}

function* leaves(node) {
  if (node.children) {
    for (const child of node.children) yield* leaves(child);
  } else {
    yield node;
  }
}

function renderTokens(root) {
  tokensEl.replaceChildren();
  for (const leaf of leaves(root)) {
    const span = document.createElement("span");
    span.className = `tok ${leaf.kind}`;
    span.textContent = showwsEl.checked ? visible(leaf.text) : leaf.text;
    span.dataset.s = leaf.start;
    span.dataset.e = leaf.end;
    tokensEl.appendChild(span);
  }
}

function kindLabel(node) {
  const frag = document.createDocumentFragment();
  const chip = document.createElement("span");
  chip.className = `kind ${node.kind}`;
  chip.textContent = node.kind;
  frag.append(chip, ` @${node.start}..${node.end}`);
  return frag;
}

function renderTreeNode(node) {
  const hasChildren = Array.isArray(node.children) && node.children.length > 0;
  const extra = node.label ?? (node.text !== undefined ? JSON.stringify(node.text) : "");
  if (!hasChildren) {
    const el = document.createElement("div");
    el.className = `leaf ${node.kind}`;
    el.append(kindLabel(node));
    if (extra) el.append("  " + extra);
    el.dataset.s = node.start;
    el.dataset.e = node.end;
    return el;
  }
  const det = document.createElement("details");
  det.open = true;
  const sum = document.createElement("summary");
  sum.append(kindLabel(node));
  if (extra) sum.append("  " + extra);
  sum.dataset.s = node.start;
  sum.dataset.e = node.end;
  det.appendChild(sum);
  for (const child of node.children) det.appendChild(renderTreeNode(child));
  return det;
}

function highlight(s, e) {
  for (const el of document.querySelectorAll("[data-s]")) {
    const inside = +el.dataset.s >= s && +el.dataset.e <= e;
    el.classList.toggle("hl", inside);
  }
}

for (const pane of [treeEl, astEl, coreEl]) {
  pane.addEventListener("mouseover", (ev) => {
    const el = ev.target.closest("[data-s]");
    if (el) highlight(+el.dataset.s, +el.dataset.e);
  });
  pane.addEventListener("mouseleave", () => highlight(-1, -1));
}

// 编辑区联动：光标所在 token ↔ 树叶
function caretHighlight() {
  const pos = srcEl.selectionStart;
  for (const el of tokensEl.querySelectorAll(".tok")) {
    if (+el.dataset.s <= pos && pos <= +el.dataset.e) {
      highlight(+el.dataset.s, +el.dataset.e);
      return;
    }
  }
}
document.addEventListener("selectionchange", () => {
  if (document.activeElement === srcEl) caretHighlight();
});

// textarea 与渲染层滚动同步
srcEl.addEventListener("scroll", () => {
  tokensEl.scrollTop = srcEl.scrollTop;
  tokensEl.scrollLeft = srcEl.scrollLeft;
});

async function loadCorpus() {
  try {
    const res = await fetch("../corpus/");
    if (!res.ok) return;
    const html = await res.text();
    const doc = new DOMParser().parseFromString(html, "text/html");
    for (const a of doc.querySelectorAll("a")) {
      const href = a.getAttribute("href");
      if (!href?.endsWith(".not")) continue;
      const opt = document.createElement("option");
      opt.value = new URL(href, res.url).href;
      opt.textContent = href.split("/").pop();
      corpusEl.appendChild(opt);
    }
  } catch {
    // 目录列表不可用时只保留手输
  }
}

corpusEl.addEventListener("change", async () => {
  if (!corpusEl.value) return;
  const res = await fetch(corpusEl.value);
  srcEl.value = await res.text();
  render();
});

function render() {
  const data = JSON.parse(analyze(srcEl.value));
  renderTokens(data.tree);
  treeEl.replaceChildren(renderTreeNode(data.tree));
  astEl.replaceChildren(renderTreeNode(data.ast));
  coreEl.replaceChildren(renderTreeNode(data.core));
}

async function main() {
  await init();
  srcEl.value = SAMPLE;
  srcEl.addEventListener("input", render);
  showwsEl.addEventListener("change", render);
  loadCorpus();
  render();
}

main();
