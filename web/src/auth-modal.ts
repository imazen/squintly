// Optional email magic-link sign-in. Pattern adapted from Weaver. Squintly
// stays anonymous-first; this opens a modal where observers can attach an
// email so they can resume on another device. Click-link, no passwords.

import { authStart } from './api';
import { getObserverId } from './conditions';

export function openSignInModal(onClose?: () => void): void {
  const scrim = document.createElement('div');
  scrim.className = 'scrim';
  scrim.innerHTML = `
    <div class="card" role="dialog" aria-modal="true" aria-labelledby="signin-title">
      <h2 id="signin-title">Save your progress</h2>
      <p class="muted" style="line-height:1.4;">Sign in with your email — we'll send a one-tap link. Anonymous use is unaffected; this just lets you resume on another device.</p>
      <div class="field">
        <label for="signin-email">Email</label>
        <input id="signin-email" type="email" inputmode="email" autocomplete="email" placeholder="you@example.com"
          style="font:inherit;color:inherit;background:#0c0c10;border:1px solid var(--border);border-radius:10px;padding:12px;width:100%;" />
      </div>
      <p id="signin-status" class="muted" style="min-height:1.2em;"></p>
      <div class="choice-row">
        <button id="signin-cancel">Not now</button>
        <button id="signin-send" class="primary">Send link</button>
      </div>
    </div>
  `;
  document.body.appendChild(scrim);

  const close = () => {
    scrim.remove();
    onClose?.();
  };

  const status = scrim.querySelector<HTMLParagraphElement>('#signin-status')!;
  const setStatus = (text: string, color = 'var(--muted)') => {
    status.style.color = color;
    status.textContent = text;
  };

  scrim.querySelector<HTMLButtonElement>('#signin-cancel')!.addEventListener('click', close);

  scrim.querySelector<HTMLButtonElement>('#signin-send')!.addEventListener('click', async () => {
    const input = scrim.querySelector<HTMLInputElement>('#signin-email')!;
    const email = input.value.trim();
    if (!email || !email.includes('@')) {
      setStatus("That doesn't look like an email address.", 'var(--warn)');
      input.focus();
      return;
    }
    setStatus('Sending…');
    try {
      const resp = await authStart({
        email,
        observer_id: getObserverId(),
        origin: location.origin,
      });
      setStatus(resp.message, 'var(--good)');
      // Close after a beat so the observer can read the confirmation.
      setTimeout(close, 2200);
    } catch (e) {
      const msg = (e as Error).message;
      // 503 from /api/auth/start ⇒ Resend not configured on this deploy.
      if (/configured|RESEND/i.test(msg)) {
        setStatus(
          'Email sign-in is not configured on this Squintly. Anonymous use still works.',
          'var(--warn)',
        );
      } else {
        setStatus(`Couldn't send the link: ${msg}`, 'var(--danger)');
      }
    }
  });
}
