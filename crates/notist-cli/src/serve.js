// Development diagnostics are isolated from book themes and authored content.
class ServeStatus extends HTMLElement {
  constructor() {
    super();
    const root = this.attachShadow({ mode: 'open' });
    const style = document.createElement('style');
    style.textContent = `
      :host {
        position: fixed; bottom: 16px; right: 16px; z-index: 2147483647;
        max-width: calc(100vw - 32px); color-scheme: dark;
        font: 14px/1.5 system-ui, sans-serif; color: #eee;
        --accent: #9cc9ff; --background: #182331;
      }
      :host([tone=error]) { --accent: #ffaaaa; --background: #2b1c20; }
      :host([tone=warning]) { --accent: #f0cd80; --background: #292419; }
      :host([tone=success]) { --accent: #a0ddb5; --background: #19291f; }
      details {
        box-sizing: border-box; border: 1px solid var(--accent); border-radius: 8px;
        background: var(--background); box-shadow: 0 4px 24px #0005;
        max-width: 100%;
      }
      details[open] { width: min(900px, calc(100vw - 32px)); }
      summary { padding: 12px 16px; cursor: pointer; color: var(--accent); }
      summary:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
      details[data-empty] summary { list-style: none; cursor: default; }
      small { margin-left: 12px; opacity: .75; white-space: nowrap; }
      .body { max-height: 40vh; overflow: auto; padding: 0 16px 12px; }
      p { margin: 0 0 12px; }
      pre {
        font: 13px/1.5 ui-monospace, monospace; white-space: pre;
        overflow-x: auto; margin: 12px 0 0; padding-top: 12px; border-top: 1px solid #ffffff26;
      }
      pre[data-severity=error] { color: #ffcccc; }
      pre[data-severity=warning] { color: #f0dcae; }
      @media (max-width: 600px) {
        :host { bottom: 8px; right: 8px; max-width: calc(100vw - 16px); }
        details[open] { width: calc(100vw - 16px); }
      }
    `;
    this.details = document.createElement('details');
    const summary = document.createElement('summary');
    this.heading = document.createElement('span');
    this.heading.setAttribute('role', 'status');
    this.duration = document.createElement('small');
    summary.append(this.heading, this.duration);
    this.body = document.createElement('div');
    this.body.className = 'body';
    this.details.append(summary, this.body);
    root.append(style, this.details);
  }

  update({ report, summary, revision }) {
    const { status, errors, warnings } = report;
    const building = status === 'building';
    const hasIssues = errors.length + warnings.length > 0;
    const wasOpen = this.details.open;
    this.setAttribute('tone', building ? 'pending'
      : errors.length ? 'error' : warnings.length ? 'warning' : 'success');
    this.heading.textContent = report.resolved && !hasIssues ? '✓ All errors resolved' : summary;
    this.duration.textContent = building ? '' : `${report.elapsed_ms}ms`;
    this.details.toggleAttribute('data-empty', !hasIssues && status !== 'failed');
    this.details.open = building ? wasOpen : errors.length > 0;
    this.body.replaceChildren();
    let note = '';
    if (building && hasIssues) note = 'Rebuilding… Previous diagnostics are being checked.';
    else if (status === 'failed') note = revision > 0
      ? 'This build failed. The page still shows the previous preview.'
      : 'No preview is available yet. Fix the errors; this page will reload automatically.';
    else if (status === 'errors') note = 'The preview was updated, but these errors still need fixing.';
    else if (report.resolved) note = 'All errors resolved.';
    if (note) {
      const paragraph = document.createElement('p');
      paragraph.textContent = note;
      this.body.append(paragraph);
    }
    for (const [severity, issues] of [['error', errors], ['warning', warnings]]) {
      for (const issue of issues) {
        const frame = document.createElement('pre');
        frame.dataset.severity = severity;
        frame.textContent = issue.detail;
        this.body.append(frame);
      }
    }
  }

  disconnected() {
    this.setAttribute('tone', 'warning');
    this.heading.textContent = 'Connection lost · retrying…';
    this.duration.textContent = '';
    const note = document.createElement('p');
    note.textContent = 'Build status is unavailable. Any diagnostics below are from the last connection.';
    this.body.prepend(note);
    this.details.removeAttribute('data-empty');
  }
}
customElements.define('notist-serve-status', ServeStatus);

const events = new EventSource(new URL('./events', import.meta.url));
const revision = Number(new URL(import.meta.url).searchParams.get('v'));
let panel;
let recoveryTimer;

function statusPanel() {
  if (!panel?.isConnected) {
    panel = document.createElement('notist-serve-status');
    document.body.append(panel);
  }
  return panel;
}

events.addEventListener('update', event => {
  const update = JSON.parse(event.data);
  clearTimeout(recoveryTimer);
  if (update.revision !== revision) { location.reload(); return; }
  const { report } = update;
  if (report.status === 'ready' && !report.resolved) {
    panel?.remove();
    return;
  }
  statusPanel().update(update);
  if (report.status === 'ready' && report.resolved) {
    recoveryTimer = setTimeout(() => panel?.remove(), 3000);
  }
});
events.addEventListener('error', () => {
  clearTimeout(recoveryTimer);
  // EventSource retries automatically. Never imply the last result is current.
  if (panel?.getAttribute('data-disconnected') === 'true') return;
  statusPanel().disconnected();
  panel.setAttribute('data-disconnected', 'true');
});
events.addEventListener('open', () => panel?.removeAttribute('data-disconnected'));
