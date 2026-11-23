/* Hue live controls. Forms are the no-JS fallback; the browser upgrades them:
 * power, brightness and colour update in place, while scene capture/apply
 * reloads to show the new record. */
(() => {
  document.documentElement.classList.add('js');
  const toast = (text, error = false) => {
    let node = document.querySelector('.toast');
    if (!node) {
      node = document.createElement('div');
      node.className = 'toast';
      document.body.appendChild(node);
    }
    node.textContent = text;
    node.classList.toggle('error', error);
    node.classList.add('show');
    clearTimeout(node._hide);
    node._hide = setTimeout(() => node.classList.remove('show'), 3800);
  };

  const post = async (form, overrides = {}) => {
    const data = new FormData(form);
    for (const [key, value] of Object.entries(overrides)) data.set(key, value);
    const response = await fetch(form.action, {
      method: 'POST',
      body: data,
      headers: { accept: 'text/html' },
    });
    if (!response.ok) {
      const detail = (await response.text()).trim();
      throw new Error(detail || `request failed (${response.status})`);
    }
    return response;
  };

  const setPower = (card, on) => {
    if (!card) return;
    card.dataset.on = on ? 'true' : 'false';
    card.classList.toggle('is-on', on);
    card.classList.toggle('is-off', !on);
    const button = card.querySelector('.switch');
    if (!button) return;
    button.classList.toggle('is-on', on);
    button.classList.toggle('is-off', !on);
    button.setAttribute('aria-pressed', on ? 'true' : 'false');
    const label = button.querySelector('.switch-label');
    if (label) label.textContent = on ? 'On' : 'Off';
  };

  const pending = new WeakMap();
  const debounce = (form, work, wait = 220) => {
    clearTimeout(pending.get(form));
    pending.set(form, setTimeout(async () => {
      try {
        await work();
      } catch (error) {
        toast(error.message || 'bridge write failed', true);
      }
    }, wait));
  };

  document.addEventListener('submit', async (event) => {
    const form = event.target.closest('form');
    if (!form) return;
    const live = form.dataset.live;
    const reload = form.dataset.reload === 'true';
    if (!live && !reload) return;
    event.preventDefault();

    const card = form.closest('.card');
    const trigger = form.querySelector('button[type="submit"]');
    if (trigger) trigger.disabled = true;

    try {
      if (live === 'power') {
        if (card) {
          const on = card.dataset.on === 'true';
          await post(form, { action: on ? 'off' : 'on' });
          setPower(card, !on);
        } else {
          await post(form);
          window.location.reload();
        }
      } else if (live === 'brightness' || live === 'colour') {
        await post(form);
      } else if (live === 'mood' || live === 'scene') {
        await post(form);
        window.location.reload();
      } else if (reload) {
        await post(form);
        window.location.reload();
      }
    } catch (error) {
      toast(error.message || 'bridge write failed', true);
    } finally {
      if (trigger) trigger.disabled = false;
    }
  });

  document.addEventListener('input', (event) => {
    const range = event.target.closest('[data-range]');
    if (range) {
      const form = range.closest('form');
      const output = form?.querySelector('output');
      if (output) output.textContent = `${range.value} / 254`;
      if (form) {
        debounce(form, async () => {
          await post(form);
        });
      }
      return;
    }

    const colour = event.target.closest('[data-color]');
    if (colour) {
      const form = colour.closest('form');
      const card = colour.closest('.card');
      const output = form?.querySelector('output');
      if (output) output.textContent = colour.value;
      if (card) card.style.setProperty('--accent', colour.value);
      if (form) {
        debounce(form, async () => {
          await post(form);
        }, 280);
      }
    }
  });
})();
