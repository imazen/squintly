// Public corpus suggestion form.
//
// Anyone can submit an image to /api/suggestions with license info, an
// original page URL, and (mandatory) email. When the visitor is signed in
// via the magic-link flow, we pre-fill `email` from observers.email and
// pass the observer_id so the backend can mark the email verified.

import { getObserverId } from './conditions';

interface LicenseOption {
  id: string;
  label: string;
  hint: string;
}

const LICENSE_OPTIONS: LicenseOption[] = [
  { id: 'self', label: 'I made this image', hint: 'You created it; you can release it.' },
  { id: 'cc0', label: 'CC0 / public domain', hint: 'Owner waived rights or it predates copyright.' },
  { id: 'cc-by', label: 'CC-BY', hint: 'Free use with attribution.' },
  { id: 'cc-by-sa', label: 'CC-BY-SA', hint: 'Free use with attribution + share-alike.' },
  { id: 'unsplash', label: 'Unsplash License', hint: 'From unsplash.com.' },
  { id: 'wikimedia-mixed', label: 'Wikimedia Commons', hint: 'See per-image terms on Commons.' },
  { id: 'owner-released', label: 'Owner released for research', hint: 'You have written permission.' },
  { id: 'fair-use-research', label: 'Fair use / research', hint: "You don't own it; submitting under research fair use." },
  { id: 'other', label: 'Other (describe in notes)', hint: '' },
];

export function startSuggest(root: HTMLElement, onExit: () => void): void {
  const observerId = getObserverId();
  const optionsHtml = LICENSE_OPTIONS.map((o) =>
    `<option value="${escapeAttr(o.id)}">${escapeHtml(o.label)}</option>`,
  ).join('');

  root.innerHTML = `
    <div class="screen suggest-screen" data-screen="suggest">
      <header class="curator-header">
        <span class="curator-title">Suggest an image</span>
        <button class="curator-exit" id="exit" aria-label="Back">×</button>
      </header>
      <p class="muted">Have an image you think should be in the corpus? Send it. Reviewer gets every submission; nothing is auto-published.</p>
      <form id="suggest-form" class="suggest-form" enctype="multipart/form-data">
        <label class="field">
          <span>Your email <strong>*</strong></span>
          <input type="email" name="email" id="email" required autocomplete="email" placeholder="you@example.com" />
          <span class="muted hint" id="email-hint">We'll only use this to follow up on your submission.</span>
        </label>
        <label class="field">
          <span>Image file <strong>*</strong></span>
          <input type="file" name="file" id="file" accept="image/*" required />
          <span class="muted hint">JPEG, PNG, WebP, AVIF, JXL, GIF, HEIC. Max 24 MB.</span>
        </label>
        <div class="suggest-preview" id="preview" hidden>
          <img id="preview-img" alt="preview" />
          <span id="preview-meta" class="muted"></span>
        </div>
        <label class="field">
          <span>Page where you found it <strong>*</strong></span>
          <input type="url" name="original_page_url" id="page" required placeholder="https://…" />
        </label>
        <label class="field">
          <span>Direct image URL (if known)</span>
          <input type="url" name="original_image_url" id="img-url" placeholder="https://…" />
        </label>
        <label class="field">
          <span>License</span>
          <select name="license_id" id="license">
            ${optionsHtml}
          </select>
          <span class="muted hint" id="license-hint"></span>
        </label>
        <label class="field">
          <span>License notes</span>
          <textarea name="license_text_freeform" id="license-text" rows="2" placeholder="Anything we should know about the rights — e.g. permission email reference, photographer release."></textarea>
        </label>
        <label class="field">
          <span>Attribution / credit</span>
          <input type="text" name="attribution" id="attribution" placeholder="Photographer name or handle" />
        </label>
        <label class="field">
          <span>Why this image</span>
          <textarea name="why" id="why" rows="2" placeholder="What edge case or content type does this cover?"></textarea>
        </label>
        <input type="hidden" name="observer_id" id="observer-id" />
        <div class="suggest-actions">
          <button type="button" id="cancel">Cancel</button>
          <button type="submit" class="primary" id="submit">Send for review</button>
        </div>
        <p class="muted suggest-disclaimer">By submitting you confirm the rights you declared above. Submissions are stored indefinitely. You can withdraw a pending submission by emailing us — we'll mark it withdrawn but the file stays for moderation review.</p>
      </form>
      <div id="suggest-result" class="suggest-result" hidden></div>
    </div>
  `;
  root.querySelector<HTMLButtonElement>('#exit')?.addEventListener('click', onExit);
  root.querySelector<HTMLButtonElement>('#cancel')?.addEventListener('click', onExit);

  const observerInput = root.querySelector<HTMLInputElement>('#observer-id')!;
  observerInput.value = observerId;
  hydrateEmailFromObserver(observerId, root);

  const license = root.querySelector<HTMLSelectElement>('#license')!;
  const licHint = root.querySelector<HTMLSpanElement>('#license-hint')!;
  const setHint = () => {
    const opt = LICENSE_OPTIONS.find((o) => o.id === license.value);
    licHint.textContent = opt?.hint ?? '';
  };
  license.addEventListener('change', setHint);
  setHint();

  const fileInput = root.querySelector<HTMLInputElement>('#file')!;
  const previewImg = root.querySelector<HTMLImageElement>('#preview-img')!;
  const previewBox = root.querySelector<HTMLDivElement>('#preview')!;
  const previewMeta = root.querySelector<HTMLSpanElement>('#preview-meta')!;
  fileInput.addEventListener('change', () => {
    const f = fileInput.files?.[0];
    if (!f) {
      previewBox.hidden = true;
      previewImg.removeAttribute('src');
      return;
    }
    if (previewImg.src) URL.revokeObjectURL(previewImg.src);
    previewImg.src = URL.createObjectURL(f);
    previewMeta.textContent = `${f.name} · ${(f.size / 1024).toFixed(0)} KB · ${f.type || 'unknown'}`;
    previewBox.hidden = false;
  });

  const form = root.querySelector<HTMLFormElement>('#suggest-form')!;
  form.addEventListener('submit', async (e) => {
    e.preventDefault();
    const submitBtn = root.querySelector<HTMLButtonElement>('#submit')!;
    submitBtn.disabled = true;
    submitBtn.textContent = 'Sending…';
    const fd = new FormData(form);
    try {
      const resp = await fetch('/api/suggestions', { method: 'POST', body: fd });
      if (!resp.ok) {
        const text = await resp.text();
        throw new Error(text || `HTTP ${resp.status}`);
      }
      const data = (await resp.json()) as { id: number; sha256: string; status: string; notification_attempted: boolean };
      const result = root.querySelector<HTMLDivElement>('#suggest-result')!;
      result.hidden = false;
      result.innerHTML = `
        <h2>Thanks — submission #${data.id} received.</h2>
        <p class="muted">sha256: <code>${escapeHtml(data.sha256)}</code></p>
        <p>${data.notification_attempted ? 'Reviewer notified by email.' : 'Reviewer will see it in the queue.'}</p>
        <p>Status: <strong>${escapeHtml(data.status)}</strong>. Nothing is published until a reviewer accepts it.</p>
        <button class="primary" id="suggest-again">Send another</button>
      `;
      form.hidden = true;
      result.querySelector<HTMLButtonElement>('#suggest-again')?.addEventListener('click', () => startSuggest(root, onExit));
    } catch (err) {
      submitBtn.disabled = false;
      submitBtn.textContent = 'Send for review';
      alert('Submission failed: ' + (err as Error).message);
    }
  });
}

async function hydrateEmailFromObserver(observerId: string, root: HTMLElement): Promise<void> {
  if (!observerId) return;
  try {
    const r = await fetch(`/api/observer/${encodeURIComponent(observerId)}/profile`);
    if (!r.ok) return;
    const data = await r.json();
    if (typeof data.email === 'string' && data.email) {
      const emailInput = root.querySelector<HTMLInputElement>('#email');
      const hint = root.querySelector<HTMLSpanElement>('#email-hint');
      if (emailInput && !emailInput.value) {
        emailInput.value = data.email;
        if (data.email_verified_at) {
          if (hint) hint.textContent = 'Pre-filled from your verified Squintly account.';
          emailInput.classList.add('verified');
        } else {
          if (hint) hint.textContent = 'Pre-filled from your Squintly account (unverified).';
        }
      }
    }
  } catch {
    // Profile endpoint failure is non-fatal; user can type the email.
  }
}

function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);
}

function escapeAttr(s: string): string {
  return escapeHtml(s);
}
