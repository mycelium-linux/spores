/* Static-surface interactions. Forms are the no-JS fallback: intercept them,
 * POST to the controller, and keep the page honest when no service is attached. */
(() => {
  const notice = (text, error) => {
    let node = document.getElementById('soil-notice');
    if (!node) {
      node = document.createElement('div');
      node.id = 'soil-notice';
      node.style.cssText = 'position:fixed;left:16px;right:16px;bottom:16px;z-index:99;padding:10px 14px;border-radius:10px;background:#1d1913;color:#f5f5f7;border:1px solid rgba(255,255,255,.16);font:14px system-ui;box-shadow:0 10px 30px rgba(0,0,0,.4)';
      document.body.appendChild(node);
    }
    node.textContent = text;
    node.style.borderColor = error ? '#7d2f2f' : 'rgba(255,255,255,.16)';
    clearTimeout(node._hide);
    node._hide = setTimeout(() => node.remove(), 4000);
  };

  document.addEventListener('submit', async (event) => {
    const form = event.target.closest('form');
    if (!form) return;
    event.preventDefault();
    const action = form.getAttribute('action');
    if (!action) return;
    try {
      const response = await fetch(action, { method: 'POST', body: new FormData(form) });
      if (response.ok) {
        window.location.reload();
        return;
      }
      notice(`controller unavailable (${response.status}); static preview`, true);
    } catch (_) {
      notice('controller unavailable; static preview', true);
    }
  });
})();
