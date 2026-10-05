(() => {
  document.querySelector('#nav-toggle')?.addEventListener('click', event => {
    const open = document.body.classList.toggle('nav-open');
    event.currentTarget.setAttribute('aria-expanded', String(open));
  });
  document.querySelector('#theme-toggle')?.addEventListener('click', () => {
    const selected = document.documentElement.dataset.theme;
    const dark = selected === 'dark' || (selected !== 'light' && matchMedia('(prefers-color-scheme:dark)').matches);
    const next = dark ? 'light' : 'dark'; document.documentElement.dataset.theme = next;
    try { localStorage.setItem('notist-theme', next); } catch (_) {}
  });
})();
