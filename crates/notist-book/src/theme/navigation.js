// Runs directly after the complete navigation, before deferred reader enhancements.
(() => {
  const navigation = document.querySelector('#book-nav');
  if (!navigation) return;
  const home = document.querySelector('link[rel="home"]')?.href || document.querySelector('.book-brand')?.href;
  const key = `notist-book-scroll:${home}`;
  let saved = null;
  let pending = null;
  try { saved = JSON.parse(sessionStorage.getItem(key)); } catch (_) {}
  const visible = () => navigation.getClientRects().length > 0;
  const offset = link => link.getBoundingClientRect().top - navigation.getBoundingClientRect().top;
  const save = (link = pending) => {
    if (!visible()) return;
    saved = { top: navigation.scrollTop };
    if (link) saved.anchor = { url: link.href, offset: offset(link) };
    try { sessionStorage.setItem(key, JSON.stringify(saved)); } catch (_) {}
  };
  const restore = () => {
    if (!visible()) return;
    if (Number.isFinite(saved?.top) && saved.top >= 0) navigation.scrollTop = saved.top;
    const anchor = saved?.anchor;
    const link = anchor && Number.isFinite(anchor.offset)
      ? [...navigation.querySelectorAll('a[href]')].find(link => link.href === anchor.url) : null;
    if (link) navigation.scrollTop += offset(link) - anchor.offset;
    const current = navigation.querySelector('[aria-current="page"]');
    if (!current) return;
    const bounds = current.getBoundingClientRect();
    const rail = navigation.getBoundingClientRect();
    const start = rail.top + (navigation.querySelector('.rail-heading')?.getBoundingClientRect().height || 0);
    if (bounds.bottom > rail.bottom) navigation.scrollTop += bounds.bottom - rail.bottom + 24;
    else if (bounds.top < start) navigation.scrollTop += bounds.top - start - 24;
  };
  navigation.addEventListener('scroll', () => save(), { passive: true });
  // Capture before the mobile reader closes the drawer. A pending link anchor must
  // survive pagehide and any scroll event caused by restoring its position.
  navigation.addEventListener('click', event => {
    const link = event.target.closest('a[href]');
    if (link && !event.defaultPrevented && event.button === 0 && !event.metaKey && !event.ctrlKey && !event.shiftKey && !event.altKey && !link.hasAttribute('download') && (!link.target || link.target === '_self')) {
      const target = new URL(link.href);
      if (target.origin === location.origin && target.pathname !== location.pathname) pending = link;
    }
    save(pending);
  }, true);
  window.addEventListener('pagehide', () => save(pending));
  // Hidden mobile drawers restore immediately when made visible, before painting.
  new MutationObserver(restore).observe(document.body, { attributes: true, attributeFilter: ['class'] });
  new ResizeObserver(restore).observe(navigation);
  restore();
})();
