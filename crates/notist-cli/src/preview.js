const events = new EventSource(new URL('./events', import.meta.url));
let revision = Number(new URL(import.meta.url).searchParams.get('v'));
events.addEventListener('update', event => {
  const update = JSON.parse(event.data);
  if (revision !== undefined && update.revision !== revision) { location.reload(); return; }
  revision = update.revision;
  document.getElementById('notist-preview-error')?.remove();
  if (!update.error) return;
  const panel = document.createElement('details');
  panel.id = 'notist-preview-error'; panel.open = true;
  panel.style.cssText = 'position:fixed;bottom:0;left:0;right:0;max-height:45vh;overflow:auto;background:#251a1a;color:#ffdddd;padding:1rem;z-index:1000;font:14px monospace';
  const title = document.createElement('summary'); title.textContent = 'Build failed';
  const body = document.createElement('pre'); body.textContent = update.error;
  panel.append(title, body); document.body.append(panel);
});
