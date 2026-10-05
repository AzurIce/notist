export const style = `
:host {
  --shader-surface: var(--pane, var(--bg, Canvas));
  --shader-text: var(--fg, CanvasText);
  --shader-muted: var(--muted, GrayText);
  --shader-line: var(--line, var(--border, color-mix(in srgb, CanvasText 14%, Canvas)));
  --shader-code: var(--code, color-mix(in srgb, var(--shader-text) 4%, var(--shader-surface)));
  --shader-accent: var(--accent, #315cc4);
  --shader-mono: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  display: block;
  margin: 1rem 0;
  color: var(--shader-text);
  font-family: inherit;
  font-size: .875rem;
  line-height: 1.5;
}
* { box-sizing: border-box; }
[hidden] { display: none !important; }
.shell {
  container: shader / inline-size;
  overflow: hidden;
  outline: 1px solid var(--shader-line);
  outline-offset: -1px;
  border-radius: 8px;
  background: var(--shader-surface);
}
.stage { position: relative; width: 100%; background: #0b0d12; }
canvas { display: block; width: 100%; height: 100%; touch-action: none; }
.toolbar { display: flex; flex-wrap: wrap; align-items: center; gap: 12px; min-height: 48px; padding: 8px 12px; }
.controls { display: flex; align-items: center; gap: 2px; }
button, summary { cursor: pointer; }
button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  min-width: 32px;
  min-height: 32px;
  border: 0;
  border-radius: 5px;
  padding: 6px 8px;
  background: transparent;
  color: inherit;
  font: inherit;
  line-height: 1;
  transition: background-color .15s, color .15s;
}
button:hover { background: var(--shader-code); }
button:disabled { cursor: default; opacity: .45; }
button:focus-visible, summary:focus-visible { outline: 2px solid var(--shader-accent); outline-offset: 2px; }
.icon { flex: none; width: 16px; height: 16px; fill: none; stroke: currentColor; stroke-width: 1.75; stroke-linecap: round; stroke-linejoin: round; }
.pause[data-paused="true"] .pause-icon, .pause[data-paused="false"] .play-icon { display: none; }
output {
  margin-left: auto;
  color: var(--shader-muted);
  white-space: nowrap;
  font: .75rem/1.5 var(--shader-mono);
  font-variant-numeric: tabular-nums;
}
.error {
  margin: 0;
  border-top: 1px solid var(--shader-line);
  padding: 12px 16px;
  background: color-mix(in srgb, #c33434 6%, var(--shader-surface));
  color: color-mix(in srgb, #c33434 80%, var(--shader-text));
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  font: .8125rem/1.65 var(--shader-mono);
}
details { border-top: 1px solid var(--shader-line); }
summary { display: flex; align-items: center; gap: 8px; min-height: 40px; padding: 8px 14px; list-style: none; }
summary::-webkit-details-marker { display: none; }
summary:hover { background: var(--shader-code); }
.chevron { width: 12px; height: 12px; color: var(--shader-muted); transition: transform .15s; }
details[open] .chevron { transform: rotate(90deg); }
.language { margin-left: auto; color: var(--shader-muted); font: .6875rem/1.5 var(--shader-mono); letter-spacing: .04em; }
.editor { border-top: 1px solid var(--shader-line); }
textarea {
  display: block;
  resize: vertical;
  width: 100%;
  min-height: 12rem;
  height: 18rem;
  max-height: 36rem;
  margin: 0;
  border: 0;
  border-radius: 0;
  padding: 14px 16px;
  background: var(--shader-code);
  color: inherit;
  caret-color: var(--shader-accent);
  tab-size: 2;
  white-space: pre;
  scrollbar-width: thin;
  scrollbar-color: var(--shader-muted) transparent;
  font: .8125rem/1.65 var(--shader-mono);
  font-variant-ligatures: none;
}
textarea:focus-visible { outline: 2px solid var(--shader-accent); outline-offset: -2px; }
.editor-footer { display: flex; align-items: center; justify-content: space-between; gap: 12px; border-top: 1px solid var(--shader-line); padding: 10px 14px; }
.apply { padding-inline: 12px; background: var(--shader-accent); color: var(--bg, Canvas); }
.apply:hover { background: color-mix(in srgb, var(--shader-accent) 88%, var(--shader-text)); }
.shortcut { color: var(--shader-muted); font: .6875rem/1.5 var(--shader-mono); }
@container shader (max-width: 420px) {
  .control .label, .shortcut { display: none; }
  .toolbar { gap: 6px; }
}
@container shader (max-width: 340px) {
  output { flex-basis: 100%; text-align: right; }
}
@media (prefers-reduced-motion: reduce) { button, .chevron { transition: none; } }
:host(:fullscreen) { width: 100%; height: 100%; margin: 0; background: var(--shader-surface); }
:host(:fullscreen) .shell { display: flex; flex-direction: column; width: 100%; height: 100%; max-width: none !important; outline: 0; border-radius: 0; }
:host(:fullscreen) .stage { flex: 1; min-height: 0; aspect-ratio: auto !important; }
:host(:fullscreen) .toolbar { flex-shrink: 0; }
:host(:fullscreen) details { max-height: 45%; overflow: auto; flex-shrink: 0; }
`;
