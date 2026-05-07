// Squintly entrypoint. Routes between welcome → calibration → profile → trials.
// Also hosts the curator-mode tab (corpus development).

import { createSession } from './api';
import { openSignInModal } from './auth-modal';
import { renderCalibration } from './calibration';
import { runCalibration } from './calibration-onboarding';
import { detectCodecs, jxlEnableHint } from './codec-probe';
import {
  captureSession,
  getObserverId,
  loadCalibration,
  loadProfile,
  saveCalibration,
  saveProfile,
  setObserverId,
  type Profile,
} from './conditions';
import { bindTabBar, renderProgressSummary, renderTabBar, startCurator } from './curator';
import { listLicenses, type LicensePolicy } from './curator-api';
import { startSuggest } from './suggest';
import { startTrials } from './trial';

const root = document.getElementById('app')!;

async function welcome(): Promise<void> {
  // Probe codec support before rendering — the result feeds the JXL banner.
  const support = await detectCodecs();
  const jxlHint = jxlEnableHint(support.supported);

  const banner = jxlHint
    ? `<p class="muted" style="background:#1a1f24;border:1px solid #2c3e50;padding:12px;border-radius:8px;line-height:1.4;">${jxlHint}</p>`
    : '';

  const progressSummary = await renderProgressSummary();

  root.innerHTML = `
    <div class="screen center" data-screen="welcome">
      ${renderTabBar('rate', { onCurator: () => {}, onRate: () => {}, onCalibrate: () => {}, onSuggest: () => {} })}
      <h1>Image Discrimination Study</h1>
      <p>You'll help <strong>make the web faster</strong>. By rating how compressed images compare to their originals, you tell us which artifacts people actually see — letting CDNs ship smaller images without anyone noticing the difference.</p>
      <p>The data trains <strong>zensim</strong>, an open-source perceptual quality metric. We especially need ratings from real phones, in real lighting, at real viewing distances — the data existing public IQA datasets don't capture.</p>
      <p>~5 minutes; the more you do, the more bytes everyone saves.</p>
      <p class="muted">No login required. We record only screen and rating data.</p>
      ${banner}
      ${progressSummary}
      <button id="begin" class="primary">Begin</button>
      <p class="muted" style="margin-top:8px;">
        <a id="signin-link" href="#" style="color:inherit;text-decoration:underline;">Already have an email-linked account? Sign in.</a>
      </p>
      <p class="muted">Best on phones, but any browser works.</p>
      <details class="credits" id="credits">
        <summary>Image sources &amp; licensing</summary>
        <div id="credits-body" class="credits-body muted">Loading…</div>
      </details>
    </div>
  `;
  bindTabBar(root, {
    onRate: () => { /* already on rate */ },
    onCurator: () => startCurator(root, () => welcome()),
    onSuggest: () => startSuggest(root, () => welcome()),
    onCalibrate: () => {
      renderCalibration(root, (result) => {
        saveCalibration(result);
        void welcome();
      });
    },
  });
  void renderCreditsBody();
  root.querySelector<HTMLAnchorElement>('#signin-link')!.addEventListener('click', (e) => {
    e.preventDefault();
    openSignInModal();
  });
  root.querySelector<HTMLButtonElement>('#begin')!.addEventListener('click', () => {
    const calib = loadCalibration();
    if (calib.css_px_per_mm == null) {
      renderCalibration(root, (result) => {
        saveCalibration(result);
        profileForm(support);
      });
    } else {
      profileForm(support);
    }
  });
}

async function renderCreditsBody(): Promise<void> {
  const host = root.querySelector<HTMLDivElement>('#credits-body');
  if (!host) return;
  try {
    const policies = await listLicenses();
    host.innerHTML = renderLicenseList(policies);
  } catch {
    host.innerHTML = `<p class="muted">License registry unavailable.</p>`;
  }
}

function renderLicenseList(policies: LicensePolicy[]): string {
  return `<table class="credits-table">
    <thead><tr><th>Source</th><th>License</th><th>Redistribute</th><th>Commercial training</th></tr></thead>
    <tbody>${policies
      .map(
        (p) => `<tr>
          <td><a href="${escapeAttr(p.terms_url)}" target="_blank" rel="noreferrer noopener" data-license-id="${escapeAttr(p.id)}">${escapeHtml(p.label)}</a><div class="muted">${escapeHtml(p.summary)}</div></td>
          <td>${escapeHtml(p.spdx_or_status)}</td>
          <td>${p.redistribute_bytes ? '✓' : '—'}</td>
          <td>${p.commercial_training ? '✓' : '—'}</td>
        </tr>`,
      )
      .join('')}</tbody></table>`;
}

function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);
}

function escapeAttr(s: string): string {
  return escapeHtml(s);
}

function profileForm(support: { supported: Set<string>; cached: boolean }): void {
  const existing = loadProfile();
  root.innerHTML = `
    <div class="screen">
      <h1>A few quick questions</h1>
      <p class="muted">All optional. Skip if you'd rather not say.</p>
      <div class="field">
        <label>Ambient light</label>
        <div class="choice-row" data-group="ambient_light">
          ${['dim', 'room', 'bright', 'outdoors'].map((v) => `<button data-v="${v}" class="${
            existing.ambient_light === v ? 'primary' : ''
          }">${v}</button>`).join('')}
        </div>
      </div>
      <div class="field">
        <label>Vision corrected?</label>
        <div class="choice-row" data-group="vision_corrected">
          ${['no', 'glasses', 'contacts'].map((v) => `<button data-v="${v}" class="${
            existing.vision_corrected === v ? 'primary' : ''
          }">${v}</button>`).join('')}
        </div>
      </div>
      <div class="field">
        <label>Age range</label>
        <div class="choice-row" data-group="age_bracket">
          ${['<25', '25-35', '35-50', '50-65', '65+'].map((v) => `<button data-v="${v}" class="${
            existing.age_bracket === v ? 'primary' : ''
          }">${v}</button>`).join('')}
        </div>
      </div>
      <div style="flex: 1"></div>
      <div class="choice-row">
        <button id="back">Back</button>
        <button id="start" class="primary">Start rating</button>
      </div>
    </div>
  `;
  const profile: Profile = { ...existing };
  for (const group of root.querySelectorAll<HTMLDivElement>('[data-group]')) {
    const key = group.dataset.group as keyof Profile;
    group.querySelectorAll<HTMLButtonElement>('button').forEach((b) => {
      b.addEventListener('click', () => {
        group.querySelectorAll('button').forEach((x) => x.classList.remove('primary'));
        b.classList.add('primary');
        profile[key] = b.dataset.v ?? null;
      });
    });
  }
  root.querySelector<HTMLButtonElement>('#back')!.addEventListener('click', () => {
    void welcome();
  });
  root.querySelector<HTMLButtonElement>('#start')!.addEventListener('click', async () => {
    saveProfile(profile);
    await beginSession(profile, support);
  });
}

async function beginSession(
  profile: Profile,
  support: { supported: Set<string>; cached: boolean },
): Promise<void> {
  const sessionConds = captureSession();
  const calib = loadCalibration();
  const observer = getObserverId();
  root.innerHTML = `<div class="screen center"><p>Starting session...</p></div>`;
  try {
    const resp = await createSession({
      observer_id: observer,
      user_agent: sessionConds.user_agent,
      age_bracket: profile.age_bracket,
      vision_corrected: profile.vision_corrected,
      device_pixel_ratio: sessionConds.device_pixel_ratio,
      screen_width_css: sessionConds.screen_width_css,
      screen_height_css: sessionConds.screen_height_css,
      color_gamut: sessionConds.color_gamut,
      dynamic_range_high: sessionConds.dynamic_range_high,
      prefers_dark: sessionConds.prefers_dark,
      pointer_type: sessionConds.pointer_type,
      timezone: sessionConds.timezone,
      viewing_distance_cm: calib.viewing_distance_cm,
      ambient_light: profile.ambient_light,
      css_px_per_mm: calib.css_px_per_mm,
      local_date: new Date().toISOString().slice(0, 10),
      supported_codecs: [...support.supported],
      codec_probe_cached: support.cached,
    });
    setObserverId(resp.observer_id);
    // Run calibration before real trials. Soft-fail allowed — even on a low
    // score we let them rate. See docs/methodology.md §3.7.
    await new Promise<void>((res) => {
      runCalibration(root, { session_id: resp.session_id, observer_id: resp.observer_id }, () => res());
    });
    const ctrl = startTrials(root, resp.session_id);
    await ctrl.start();
    window.addEventListener('beforeunload', () => {
      navigator.sendBeacon(`/api/session/${encodeURIComponent(resp.session_id)}/end`);
    });
  } catch (e) {
    root.innerHTML = `<div class="screen center"><h1>Couldn't start</h1><p class="muted">${(e as Error).message}</p><button class="primary" onclick="location.reload()">Retry</button></div>`;
  }
}

welcome();
