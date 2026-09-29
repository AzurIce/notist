import init, { analyze } from "./pkg/notist.js";

const srcEl = document.querySelector("#src");
const tokensEl = document.querySelector("#tokens");
const treeEl = document.querySelector("#tree");
const showwsEl = document.querySelector("#showws");

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
  if (!node.children) {
    const el = document.createElement("div");
    el.className = `leaf ${node.kind}`;
    el.append(kindLabel(node), `  ${JSON.stringify(node.text)}`);
    el.dataset.s = node.start;
    el.dataset.e = node.end;
    return el;
  }
  const det = document.createElement("details");
  det.open = true;
  const sum = document.createElement("summary");
  sum.append(kindLabel(node));
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

treeEl.addEventListener("mouseover", (ev) => {
  const el = ev.target.closest("[data-s]");
  if (el) highlight(+el.dataset.s, +el.dataset.e);
});
treeEl.addEventListener("mouseleave", () => highlight(-1, -1));

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

function render() {
  const data = JSON.parse(analyze(srcEl.value));
  renderTokens(data.tree);
  treeEl.replaceChildren(renderTreeNode(data.tree));
}

async function main() {
  await init();
  srcEl.value = SAMPLE;
  srcEl.addEventListener("input", render);
  showwsEl.addEventListener("change", render);
  render();
}

main();
