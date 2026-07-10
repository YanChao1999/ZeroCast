(function () {
  const snippets = {
    build: document.getElementById('code-build')?.textContent ?? '',
    cast: document.getElementById('code-cast')?.textContent ?? '',
  };

  function resetCopyBtn(btn, delayMs) {
    clearTimeout(btn._copyResetTimer);
    btn._copyResetTimer = setTimeout(() => {
      btn.textContent = 'Copy';
      btn.classList.remove('copied');
    }, delayMs);
  }

  document.querySelectorAll('.copy-btn').forEach((btn) => {
    btn.addEventListener('click', async () => {
      const key = btn.dataset.copy;
      const text = snippets[key];
      if (!text) return;

      try {
        await navigator.clipboard.writeText(text.trim());
        btn.textContent = 'Copied!';
        btn.classList.add('copied');
        resetCopyBtn(btn, 2000);
      } catch {
        btn.textContent = 'Failed';
        btn.classList.remove('copied');
        resetCopyBtn(btn, 2000);
      }
    });
  });

  const toggle = document.querySelector('.nav-toggle');
  const links = document.querySelector('.nav-links');

  toggle?.addEventListener('click', () => {
    const open = links?.classList.toggle('open');
    toggle.setAttribute('aria-expanded', String(!!open));
  });

  links?.querySelectorAll('a').forEach((link) => {
    link.addEventListener('click', () => {
      links.classList.remove('open');
      toggle?.setAttribute('aria-expanded', 'false');
    });
  });
})();
