// Squintly entrypoint. Routes between welcome → calibration → profile → trials.
// Also hosts the curator-mode tab (corpus development).

import { createSession, listStudies, type Study } from './api';
import { openSignInModal } from './auth-modal';
import { renderCalibration } from './calibration';
import { runCalibration } from './calibration-onboarding';
import { detectCodecs, jxlEnableHint } from './codec-probe';
import {
  captureSession,
  getObserverId,
  loadCalibration,
  loadProfile,
  loadStudyId,
  saveCalibration,
  saveProfile,
  saveStudyId,
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

  // Studies are fetched rather than hard-coded so the picker can never drift
  // from what the server will actually accept.
  let studies: Study[] = [];
  try {
    studies = await listStudies();
  } catch {
    // Offer nothing rather than a stale guess; the server default applies.
  }
  const chosen = pickStudy(studies, loadStudyId());
  const studyPicker = studies.length > 1 ? renderStudyPicker(studies, chosen) : '';

  root.innerHTML = `
    <div class="screen center" data-screen="welcome">
      ${renderTabBar('rate', { onCurator: () => {}, onRate: () => {}, onCalibrate: () => {}, onSuggest: () => {} })}
      <h1>Image Discrimination Study</h1>
      <p>You'll help <strong>make the web faster</strong>. By rating how compressed images compare to their originals, you tell us which artifacts people actually see — letting CDNs ship smaller images without anyone noticing the difference.</p>
      <p>The data trains <strong>zensim</strong>, an open-source perceptual quality metric. We especially need ratings from real phones, in real lighting, at real viewing distances — the data existing public IQA datasets don't capture.</p>
      <p>~5 minutes; the more you do, the more bytes everyone saves.</p>
      <p class="muted">No login required. We record only screen and rating data.</p>
      ${banner}
      ${studyPicker}
      ${progressSummary}
      <button id="begin" class="primary">Begin</button>
      <p class="muted" style="margin-top:8px;">
        <a id="signin-link" href="#" style="color:inherit;text-decoration:underline;">Already have an email-linked account? Sign in.</a>
      </p>
      <p class="muted" style="margin-top:0;">
        <a id="calibrate-link" href="#" style="color:inherit;text-decoration:underline;">${
          loadCalibration().css_px_per_mm
            ? 'Screen size calibrated \u2713 \u2014 re-measure'
            : 'Calibrate screen size (optional)'
        }</a>
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
    onCalibrate: () => openCalibration(),
  });
  root.querySelector<HTMLAnchorElement>('#calibrate-link')!.addEventListener('click', (e) => {
    e.preventDefault();
    openCalibration();
  });
  bindStudyPicker(root, studies);
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


/**
 * Resolve which study is selected: the stored choice if the server still
 * offers it, otherwise the first listed one. A stored id that has since been
 * retired must not be sent — the server rejects unknown ids rather than
 * silently substituting, which is right, but the UI shouldn't provoke it.
 */
/// Calibration is a one-off measurement the app remembers, so it is reached
/// deliberately rather than sitting in the tab bar. `renderCalibration` seeds
/// itself from the stored value and its Skip preserves it, so re-entering here
/// can only improve the number, never blank it.
function openCalibration(): void {
  renderCalibration(root, (result) => {
    saveCalibration(result);
    void /// Is this a returning observer who has already been through onboarding?
///
/// Reopening the app used to drop everyone back on the welcome screen and walk
/// them through Begin -> calibration -> profile again, even though the observer
/// id, profile and calibration were all already in localStorage. For someone
/// coming back for a second batch of trials that is three screens of friction
/// in front of the thing they returned to do.
///
/// "Already onboarded" means we have an observer id and a saved profile — the
/// two things the profile step exists to produce. Calibration is deliberately
/// not required: it is optional and skippable, so demanding it would make the
/// resume path stricter than the path that created the state.
function hasOnboarded(): boolean {
  const p = loadProfile();
  return (
    !!getObserverId() &&
    (p.ambient_light !== null || p.vision_corrected !== null || p.age_bracket !== null)
  );
}

async function boot(): Promise<void> {
  if (!hasOnboarded()) {
    await welcome();
    return;
  }
  // Straight back into trials. A new session row is correct rather than
  // resuming the old one: conditions are re-captured, and the screen, lighting
  // or device may well have changed since last time — pretending otherwise
  // would file new responses under stale viewing conditions.
  root.innerHTML = `
    <div class="screen center" data-screen="resuming">
      <div class="spinner" role="status" aria-label="Resuming"></div>
      <p class="muted">Welcome back — picking up where you left off…</p>
      <p class="muted"><a id="not-now" href="#" style="color:inherit;text-decoration:underline;">Start from the beginning instead</a></p>
    </div>
  `;
  let cancelled = false;
  root.querySelector<HTMLAnchorElement>('#not-now')!.addEventListener('click', (e) => {
    e.preventDefault();
    cancelled = true;
    void welcome();
  });
  const support = await detectCodecs();
  if (cancelled) return;
  await beginSession(loadProfile(), support, { keepScreen: true });
}

void boot();
  });
}

function pickStudy(studies: Study[], stored: string | null): Study | null {
  if (!studies.length) return null;
  return studies.find((s) => s.id === stored) ?? studies[0];
}

function renderStudyPicker(studies: Study[], chosen: Study | null): string {
  return `
    <div class="study-picker" id="study-picker">
      <div class="study-picker-label muted">Choose what to help with</div>
      ${studies
        .map(
          (s) => `
        <button class="study-option${s.id === chosen?.id ? ' on' : ''}" data-study="${escapeAttr(s.id)}">
          <span class="study-name">${escapeHtml(s.label)}</span>
          <span class="study-summary muted">${escapeHtml(s.summary)}</span>
          <span class="study-style">${escapeHtml(s.trial_style)}</span>
        </button>`,
        )
        .join('')}
    </div>`;
}

function bindStudyPicker(root: HTMLElement, studies: Study[]): void {
  if (studies.length < 2) return;
  root.querySelectorAll<HTMLButtonElement>('.study-option').forEach((b) => {
    b.addEventListener('click', () => {
      const id = b.dataset.study;
      if (!id) return;
      saveStudyId(id);
      root.querySelectorAll('.study-option').forEach((x) => x.classList.remove('on'));
      b.classList.add('on');
    });
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
    void /// Is this a returning observer who has already been through onboarding?
///
/// Reopening the app used to drop everyone back on the welcome screen and walk
/// them through Begin -> calibration -> profile again, even though the observer
/// id, profile and calibration were all already in localStorage. For someone
/// coming back for a second batch of trials that is three screens of friction
/// in front of the thing they returned to do.
///
/// "Already onboarded" means we have an observer id and a saved profile — the
/// two things the profile step exists to produce. Calibration is deliberately
/// not required: it is optional and skippable, so demanding it would make the
/// resume path stricter than the path that created the state.
function hasOnboarded(): boolean {
  const p = loadProfile();
  return (
    !!getObserverId() &&
    (p.ambient_light !== null || p.vision_corrected !== null || p.age_bracket !== null)
  );
}

async function boot(): Promise<void> {
  if (!hasOnboarded()) {
    await welcome();
    return;
  }
  // Straight back into trials. A new session row is correct rather than
  // resuming the old one: conditions are re-captured, and the screen, lighting
  // or device may well have changed since last time — pretending otherwise
  // would file new responses under stale viewing conditions.
  root.innerHTML = `
    <div class="screen center" data-screen="resuming">
      <div class="spinner" role="status" aria-label="Resuming"></div>
      <p class="muted">Welcome back — picking up where you left off…</p>
      <p class="muted"><a id="not-now" href="#" style="color:inherit;text-decoration:underline;">Start from the beginning instead</a></p>
    </div>
  `;
  let cancelled = false;
  root.querySelector<HTMLAnchorElement>('#not-now')!.addEventListener('click', (e) => {
    e.preventDefault();
    cancelled = true;
    void welcome();
  });
  const support = await detectCodecs();
  if (cancelled) return;
  await beginSession(loadProfile(), support, { keepScreen: true });
}

void boot();
  });
  root.querySelector<HTMLButtonElement>('#start')!.addEventListener('click', async () => {
    saveProfile(profile);
    await beginSession(profile, support);
  });
}

async function beginSession(
  profile: Profile,
  support: { supported: Set<string>; cached: boolean },
  opts: { keepScreen?: boolean } = {},
): Promise<void> {
  const sessionConds = captureSession();
  const calib = loadCalibration();
  const observer = getObserverId();
  // On resume the caller already has a screen up that carries the only way out
  // ("start from the beginning instead"). Replacing it with a placeholder would
  // remove that escape while the request is in flight — which is exactly when
  // someone who opened the app by accident wants it.
  if (!opts.keepScreen) {
    root.innerHTML = `<div class="screen center"><p>Starting session...</p></div>`;
  }
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
      study_id: loadStudyId(),
    });
    setObserverId(resp.observer_id);
    // Record what the server actually joined us to, so a retired study id in
    // localStorage self-heals instead of failing on every future session.
    saveStudyId(resp.study_id);
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

/// Is this a returning observer who has already been through onboarding?
///
/// Reopening the app used to drop everyone back on the welcome screen and walk
/// them through Begin -> calibration -> profile again, even though the observer
/// id, profile and calibration were all already in localStorage. For someone
/// coming back for a second batch of trials that is three screens of friction
/// in front of the thing they returned to do.
///
/// "Already onboarded" means we have an observer id and a saved profile — the
/// two things the profile step exists to produce. Calibration is deliberately
/// not required: it is optional and skippable, so demanding it would make the
/// resume path stricter than the path that created the state.
function hasOnboarded(): boolean {
  const p = loadProfile();
  return (
    !!getObserverId() &&
    (p.ambient_light !== null || p.vision_corrected !== null || p.age_bracket !== null)
  );
}

async function boot(): Promise<void> {
  if (!hasOnboarded()) {
    await welcome();
    return;
  }
  // Straight back into trials. A new session row is correct rather than
  // resuming the old one: conditions are re-captured, and the screen, lighting
  // or device may well have changed since last time — pretending otherwise
  // would file new responses under stale viewing conditions.
  root.innerHTML = `
    <div class="screen center" data-screen="resuming">
      <div class="spinner" role="status" aria-label="Resuming"></div>
      <p class="muted">Welcome back — picking up where you left off…</p>
      <p class="muted"><a id="not-now" href="#" style="color:inherit;text-decoration:underline;">Start from the beginning instead</a></p>
    </div>
  `;
  let cancelled = false;
  root.querySelector<HTMLAnchorElement>('#not-now')!.addEventListener('click', (e) => {
    e.preventDefault();
    cancelled = true;
    void welcome();
  });
  const support = await detectCodecs();
  if (cancelled) return;
  await beginSession(loadProfile(), support, { keepScreen: true });
}

void boot();
