(() => {
  const root = document.documentElement;

  // Follow the system until the reader makes an explicit choice. Storage is optional.
  const systemTheme = matchMedia('(prefers-color-scheme: dark)');
  const themeToggle = document.querySelector('#theme-toggle');
  const isDark = () => root.dataset.theme === 'dark' ||
    (root.dataset.theme !== 'light' && systemTheme.matches);
  const updateThemeButton = () => {
    const label = `Switch to ${isDark() ? 'light' : 'dark'} theme`;
    themeToggle?.setAttribute('aria-label', label);
    themeToggle?.setAttribute('title', label);
    const text = themeToggle?.querySelector('.theme-label');
    if (text) text.textContent = isDark() ? 'Light' : 'Dark';
  };
  themeToggle?.addEventListener('click', () => {
    root.dataset.theme = isDark() ? 'light' : 'dark';
    try { localStorage.setItem('notist-theme', root.dataset.theme); } catch (_) {}
    updateThemeButton();
  });
  systemTheme.addEventListener('change', updateThemeButton);
  window.addEventListener('storage', event => {
    if (event.key === 'notist-theme' || event.key === null) {
      root.dataset.theme = event.newValue === 'light' || event.newValue === 'dark'
        ? event.newValue : 'auto';
      updateThemeButton();
    }
  });
  updateThemeButton();

  // The mobile contents behave as a modal drawer, including for keyboard users.
  const mobile = matchMedia('(max-width: 760px)');
  const navigation = document.querySelector('#book-nav');
  const bookIdentity = document.querySelector('.book-brand')?.href || location.href;

  // Keep the reader's preferred width even when a smaller viewport temporarily clamps it.
  const resizeHandle = document.querySelector('#nav-resize');
  const layout = document.querySelector('.book-layout');
  if (navigation && resizeHandle && layout) {
    const widthKey = `notist-book-width:${bookIdentity}`;
    let preferredWidth = null;
    let drag = null;
    let resizeFocused = false;
    const widthLimits = () => {
      const style = getComputedStyle(layout);
      const min = parseFloat(style.getPropertyValue('--navigation-min-width'));
      const max = parseFloat(style.getPropertyValue('--navigation-max-width'));
      const reserve = parseFloat(style.getPropertyValue('--navigation-content-space'));
      return { min, max, available: Math.max(min, Math.min(max, layout.clientWidth - reserve)) };
    };
    const updateResizeHandle = () => {
      if (mobile.matches) return;
      const { min, available } = widthLimits();
      resizeHandle.setAttribute('aria-valuemin', String(min));
      resizeHandle.setAttribute('aria-valuemax', String(Math.floor(available)));
      resizeHandle.setAttribute('aria-valuenow', String(Math.round(navigation.getBoundingClientRect().width)));
    };
    const applyWidth = () => {
      if (preferredWidth === null) root.style.removeProperty('--navigation-user-width');
      else root.style.setProperty('--navigation-user-width', `${preferredWidth}px`);
      updateResizeHandle();
    };
    const loadWidth = () => {
      preferredWidth = null;
      try {
        const stored = localStorage.getItem(widthKey);
        const value = Number(stored);
        if (stored !== null && Number.isFinite(value) && value > 0) {
          const { min, max } = widthLimits();
          preferredWidth = Math.max(min, Math.min(max, value));
        }
      } catch (_) {}
      applyWidth();
    };
    const saveWidth = () => {
      try {
        if (preferredWidth === null) localStorage.removeItem(widthKey);
        else localStorage.setItem(widthKey, String(preferredWidth));
      } catch (_) {}
    };
    const setWidth = width => {
      const { min, available } = widthLimits();
      preferredWidth = Math.round(Math.max(min, Math.min(available, width)));
      applyWidth();
    };
    const finishDrag = (commit = true) => {
      if (!drag) return;
      const previous = drag;
      drag = null;
      root.classList.remove('nav-resizing');
      if (resizeHandle.hasPointerCapture(previous.id)) resizeHandle.releasePointerCapture(previous.id);
      if (commit) saveWidth();
      else {
        preferredWidth = previous.preferredWidth;
        applyWidth();
      }
    };
    const resetWidth = () => {
      finishDrag(false);
      preferredWidth = null;
      applyWidth();
      saveWidth();
    };
    resizeHandle.addEventListener('pointerdown', event => {
      if (event.button !== 0 || !event.isPrimary || mobile.matches) return;
      drag = { id: event.pointerId, x: event.clientX, width: navigation.getBoundingClientRect().width, preferredWidth };
      resizeHandle.setPointerCapture(event.pointerId);
      resizeHandle.focus({ preventScroll: true });
      root.classList.add('nav-resizing');
      event.preventDefault();
    });
    resizeHandle.addEventListener('pointermove', event => {
      if (drag?.id === event.pointerId) setWidth(drag.width + event.clientX - drag.x);
    });
    resizeHandle.addEventListener('pointerup', event => {
      if (drag?.id === event.pointerId) finishDrag();
    });
    resizeHandle.addEventListener('pointercancel', event => {
      if (drag?.id === event.pointerId) finishDrag(false);
    });
    resizeHandle.addEventListener('lostpointercapture', () => finishDrag(!mobile.matches));
    resizeHandle.addEventListener('dblclick', resetWidth);
    resizeHandle.addEventListener('focus', () => { resizeFocused = true; });
    resizeHandle.addEventListener('blur', () => {
      // A media-query display change can blur the handle before its change event runs.
      if (!mobile.matches) resizeFocused = false;
    });
    resizeHandle.addEventListener('keydown', event => {
      if (event.key === 'Home') {
        event.preventDefault();
        resetWidth();
      } else if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
        event.preventDefault();
        setWidth(navigation.getBoundingClientRect().width + (event.key === 'ArrowRight' ? 16 : -16));
        saveWidth();
      }
    });
    mobile.addEventListener('change', () => {
      finishDrag(false);
      if (mobile.matches && resizeFocused) {
        document.querySelector('#nav-toggle')?.focus({ preventScroll: true });
        resizeFocused = false;
      }
      updateResizeHandle();
    });
    window.addEventListener('storage', event => {
      if (event.key === widthKey || event.key === null) {
        finishDrag(false);
        loadWidth();
      }
    });
    const widthObserver = new ResizeObserver(updateResizeHandle);
    widthObserver.observe(layout);
    widthObserver.observe(navigation);
    loadWidth();
  }

  const navToggle = document.querySelector('#nav-toggle');
  const navClose = document.querySelector('#nav-close');
  const backdrop = document.querySelector('#nav-backdrop');
  const background = [...document.querySelectorAll('.book-header, #page-content, .page-outline, .book-footer')];
  const previousInert = new Map();
  let returnFocus = null;
  let originalRole = null;
  const setNavOpen = (open, restoreFocus = true) => {
    if (!navigation || !navToggle) return;
    open = open && mobile.matches;
    if (open === document.body.classList.contains('nav-open')) return;
    document.body.classList.toggle('nav-open', open);
    navToggle.setAttribute('aria-expanded', String(open));
    navToggle.setAttribute('aria-label', open ? 'Close contents' : 'Open contents');
    if (backdrop) backdrop.hidden = !open;
    if (open) {
      returnFocus = document.activeElement;
      originalRole = navigation.getAttribute('role');
      navigation.setAttribute('role', 'dialog');
      navigation.setAttribute('aria-modal', 'true');
      for (const element of background) {
        previousInert.set(element, element.inert);
        element.inert = true;
      }
      (navClose || navigation.querySelector('a, button'))?.focus({ preventScroll: true });
    } else {
      if (originalRole === null) navigation.removeAttribute('role');
      else navigation.setAttribute('role', originalRole);
      navigation.removeAttribute('aria-modal');
      for (const [element, inert] of previousInert) element.inert = inert;
      previousInert.clear();
      if (restoreFocus && returnFocus instanceof HTMLElement) returnFocus.focus();
    }
  };
  navToggle?.addEventListener('click', () => setNavOpen(!document.body.classList.contains('nav-open')));
  navClose?.addEventListener('click', () => setNavOpen(false));
  backdrop?.addEventListener('click', () => setNavOpen(false));
  navigation?.addEventListener('click', event => {
    if (event.target.closest('a')) {
      setNavOpen(false, false);
    }
  });
  mobile.addEventListener('change', () => {
    const wasCloseFocused = document.activeElement === navClose;
    setNavOpen(false, false);
    if (wasCloseFocused) navigation?.querySelector('[aria-current="page"], a')?.focus({ preventScroll: true });
  });
  document.addEventListener('keydown', event => {
    if (!document.body.classList.contains('nav-open')) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      setNavOpen(false);
    } else if (event.key === 'Tab') {
      const focusable = [...navigation.querySelectorAll('a[href], summary, button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])')]
        .filter(element => element.checkVisibility() && !element.closest('[inert]'));
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last?.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first?.focus();
      }
    }
  });

  // One scroll calculation drives both the outline and progress, at most once per frame.
  const article = document.querySelector('.prose');
  const progress = document.querySelector('#reading-progress span');
  const outlineLinks = [...document.querySelectorAll('[data-outline-link]')];
  const headings = [];
  const linksByTarget = new Map();
  for (const link of outlineLinks) {
    let target;
    try { target = document.getElementById(decodeURIComponent(link.hash.slice(1))); } catch (_) { continue; }
    if (!target) continue;
    if (!linksByTarget.has(target)) {
      linksByTarget.set(target, []);
      // Explicit IDs can belong to a section; track its heading, not its full height.
      headings.push({ target, heading: target.matches('section') ? target.querySelector('h1, h2, h3, h4, h5, h6, [role="heading"]') || target : target });
    }
    linksByTarget.get(target).push(link);
  }
  let activeTarget = null;
  let scheduled = false;
  const updateReading = () => {
    scheduled = false;
    if (!article) return;
    const headerHeight = document.querySelector('.book-header')?.getBoundingClientRect().height || 0;
    const bounds = article.getBoundingClientRect();
    const distance = bounds.height - (window.innerHeight - headerHeight);
    const fraction = distance <= 0 ? 0 : Math.min(1, Math.max(0, (headerHeight - bounds.top) / distance));
    if (progress) progress.style.transform = `scaleX(${fraction})`;
    let current = headings[0]?.target;
    for (const { target, heading } of headings) {
      if (heading.getBoundingClientRect().top <= headerHeight + 40) current = target;
    }
    if (current !== activeTarget) {
      for (const link of linksByTarget.get(activeTarget) || []) link.removeAttribute('aria-current');
      for (const link of linksByTarget.get(current) || []) link.setAttribute('aria-current', 'location');
      activeTarget = current;
    }
  };
  const scheduleReading = () => {
    if (!scheduled) {
      scheduled = true;
      requestAnimationFrame(updateReading);
    }
  };
  window.addEventListener('scroll', scheduleReading, { passive: true });
  window.addEventListener('resize', scheduleReading);
  window.addEventListener('load', scheduleReading);
  if (article && typeof ResizeObserver !== 'undefined') new ResizeObserver(scheduleReading).observe(article);
  document.fonts?.ready.then(scheduleReading);
  for (const link of outlineLinks) {
    link.addEventListener('click', () => {
      const details = link.closest('details');
      if (details) details.open = false;
    });
  }
  scheduleReading();

  // Components can expose plain pre/code in light DOM and opt into the same raw UI.
  const enhanceCodeBlock = (pre, dynamicallyAdded = false) => {
    if (!pre.closest('.prose')) return;
    const code = pre.firstElementChild;
    if (code?.tagName !== 'CODE' || pre.closest('.code-block')) return;
    // Keep enhancing all server-rendered raw content, including mounted component children.
    // Newly generated component UI is left alone unless it explicitly opts in.
    if (dynamicallyAdded && pre.closest('[notist-protocol]') && !pre.hasAttribute('data-notist-code-block')) return;
    const block = document.createElement('div');
    block.className = 'code-block';
    const toolbar = document.createElement('div');
    toolbar.className = 'code-toolbar';
    const language = document.createElement('span');
    language.className = 'code-language';
    language.textContent = [...code.classList].find(name => name.startsWith('language-'))?.slice(9) || 'Code';
    const copy = document.createElement('button');
    copy.type = 'button';
    copy.className = 'copy-code';
    copy.textContent = 'Copy';
    copy.setAttribute('aria-label', 'Copy code');
    copy.setAttribute('aria-live', 'polite');
    let reset;
    copy.addEventListener('click', async () => {
      clearTimeout(reset);
      try {
        if (!navigator.clipboard?.writeText) throw new Error('Clipboard unavailable');
        await navigator.clipboard.writeText(code.textContent);
        copy.textContent = 'Copied';
        copy.setAttribute('aria-label', 'Code copied');
      } catch (_) {
        // Do not pretend a denied clipboard operation succeeded. The code stays selectable.
        copy.textContent = 'Select to copy';
        copy.setAttribute('aria-label', 'Clipboard unavailable; select the code to copy');
        const selection = window.getSelection();
        const range = document.createRange();
        range.selectNodeContents(code);
        selection?.removeAllRanges();
        selection?.addRange(range);
      }
      reset = setTimeout(() => {
        copy.textContent = 'Copy';
        copy.setAttribute('aria-label', 'Copy code');
      }, 2500);
    });
    toolbar.append(language, copy);
    // The wrapper takes over a component's named slot without moving its source out of the host.
    if (pre.hasAttribute('slot')) {
      block.setAttribute('slot', pre.getAttribute('slot'));
      pre.removeAttribute('slot');
    }
    pre.before(block);
    block.append(toolbar, pre);
  };
  for (const pre of document.querySelectorAll('.prose pre')) enhanceCodeBlock(pre);
  if (article) {
    new MutationObserver(records => {
      for (const record of records) {
        for (const node of record.addedNodes) {
          if (!(node instanceof Element)) continue;
          if (node.matches('pre')) enhanceCodeBlock(node, true);
          for (const pre of node.querySelectorAll('pre')) enhanceCodeBlock(pre, true);
        }
      }
    }).observe(article, { childList: true, subtree: true });
  }
})();
