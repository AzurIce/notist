import init, { analyze } from "./pkg/notist.js";

const srcEl = document.querySelector("#src");
const tokensEl = document.querySelector("#tokens");
const treeEl = document.querySelector("#tree");
const astEl = document.querySelector("#ast");
const ir1El = document.querySelector("#ir1");
const ir2El = document.querySelector("#ir2");
const coreEl = document.querySelector("#core");
const diagsEl = document.querySelector("#diags");
const showwsEl = document.querySelector("#showws");
const corpusEl = document.querySelector("#corpus");

const SAMPLE = [
  "= notist 一览",
  "",
  "@(tags: (\"demo\", \"ui\"))",
  "#strong[行内 *strong* 与 _emph_，转义 \\* 所见即所得]",
  "",
  "- 列表 *甲*",
  "- 列表乙",
  "",
  "#list[",
  "块 body 第一段",
  "",
  "- 块内列表",
  "]",
].join("\n");

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

const syncPanes = [treeEl, astEl, ir1El, ir2El, coreEl];

function highlight(s, e) {
  for (const el of document.querySelectorAll("[data-s]")) {
    const inside = +el.dataset.s >= s && +el.dataset.e <= e;
    el.classList.toggle("hl", inside);
  }
  // 无包含命中时（点 span、单字符 token）：退化为各面板的最小包含节点
  for (const pane of [...syncPanes, tokensEl]) {
    if (pane.querySelector(".hl")) continue;
    let best = null;
    for (const el of pane.querySelectorAll("[data-s]")) {
      const es = +el.dataset.s;
      const ee = +el.dataset.e;
      if (es <= s && e <= ee && (!best || ee - es < best.span)) {
        best = { el, span: ee - es };
      }
    }
    // 根节点（Document）不算——命中它等于什么也没命中
    if (best && best.el.parentElement.closest("[data-s]")) {
      best.el.classList.add("hl");
    }
  }
  revealSync();
}

function revealIn(container, el) {
  const c = container.getBoundingClientRect();
  const r = el.getBoundingClientRect();
  if (r.top < c.top) container.scrollTop -= c.top - r.top;
  else if (r.bottom > c.bottom) container.scrollTop += r.bottom - c.bottom;
}

function revealSync() {
  for (const pane of syncPanes) {
    const target = pane.querySelector(".hl");
    if (!target) continue;
    for (let p = target.parentElement; p && p !== pane; p = p.parentElement) {
      if (p.tagName === "DETAILS") p.open = true;
    }
    revealIn(pane, target);
  }
  const tok = tokensEl.querySelector(".hl");
  if (tok) {
    revealIn(tokensEl, tok);
    srcEl.scrollTop = tokensEl.scrollTop;
    srcEl.scrollLeft = tokensEl.scrollLeft;
  }
}

for (const pane of [...syncPanes, diagsEl]) {
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
  ir1El.replaceChildren(renderTreeNode(data.ir1));
  ir2El.replaceChildren(renderTreeNode(data.ir2));
  coreEl.replaceChildren(renderTreeNode(data.core));
  renderDiags(data.diagnostics);
}

function renderDiags(diags) {
  diagsEl.replaceChildren();
  if (!diags.length) {
    const ok = document.createElement("div");
    ok.className = "ok";
    ok.textContent = "✓ 无诊断";
    diagsEl.appendChild(ok);
    return;
  }
  for (const d of diags) {
    const el = document.createElement("div");
    el.className = `diag ${d.phase}`;
    el.dataset.s = d.start;
    el.dataset.e = d.end;
    el.textContent = `error[${d.phase}] @${d.start}..${d.end}: ${d.message}`;
    diagsEl.appendChild(el);
  }
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
