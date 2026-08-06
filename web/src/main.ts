// Squintly entrypoint. Routes between welcome → calibration → profile → trials.
// Also hosts the curator-mode tab (corpus development).

import { createSession, listStudies, signOut, type Study } from './api';
import { openSignInModal } from './auth-modal';
import { renderCalibration } from './calibration';
import { runCalibration } from './calibration-onboarding';
import { hasChosenInputMode } from './input-mode';
import { maybeShowInstructions } from './instructions';
import { renderLanding } from './landing';
import { maybeShowDebrief } from './debrief';
import { renderBoard } from './board';
import { showAdmin } from './admin';
import { chooseInputMode } from './mode-chooser';
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
    void boot();
  });
}

/// Which study is selected: the stored choice, else the one flagged default.
///
/// `studies[0]` was the fallback, which is declaration order — so the picker
/// preselected "Web image quality" while the server's default was
/// `ssim2-nonphoto`, and an observer who never touched the picker was offered
/// one study and enrolled in another. `is_default` is asserted unique in
/// `studies.rs`, so this cannot drift again.
function pickStudy(studies: Study[], stored: string | null): Study | null {
  if (!studies.length) return null;
  return (
    studies.find((s) => s.id === stored) ?? studies.find((s) => s.is_default) ?? studies[0]
  );
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
    void boot();
  });
  root.querySelector<HTMLButtonElement>('#start')!.addEventListener('click', async () => {
    saveProfile(profile);
    // Last screen before the first trial, which is the only place a how-to for
    // the gesture is actually read.
    if (!hasChosenInputMode()) await chooseInputMode(root);
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
    const ctrl = startTrials(root, resp.session_id, {
      // A session belongs to one study, so switching means ending this one and
      // starting a fresh session under the new choice — not mutating this one,
      // which would leave its trials filed under a study the observer left.
      onSwitchStudy: () => {
        navigator.sendBeacon(`/api/session/${encodeURIComponent(resp.session_id)}/end`);
        void beginSession(profile, support);
      },
      onRecalibrate: () => {
        navigator.sendBeacon(`/api/session/${encodeURIComponent(resp.session_id)}/end`);
        renderCalibration(root, (result) => {
          saveCalibration(result);
          // Straight back into trials: the observer came from a trial and did
          // not ask to go to the welcome screen. Conditions are re-captured by
          // the new session, which is what makes the new measurement count.
          void beginSession(profile, support);
        });
      },
    });
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

/// Where the session lives.
///
/// A separate URL, not a screen behind a button. Two reasons, and the second is
/// the one that mattered in practice:
///
///  * The front page and the study are different things to link to. "Come and
///    help" wants `/`; "carry on where you left off" wants `/rate`.
///  * Everything that drives the app — a test, a bookmark, a reload mid-session
///    — could otherwise only reach the study by simulating a click through the
///    front page. When the front page moved in front of everything, ~40 e2e
///    specs broke on exactly that, and the fix was a helper that clicked past
///    it everywhere. A URL is the honest version of that helper.
export const RATE_PATH = '/rate';
/// The reviewer board. Its own route so it can be linked and returned to.
export const BOARD_PATH = '/board';
/// The operator view. A route rather than a menu item so an admin can reach it
/// from the front page without starting a session first.
export const ADMIN_PATH = '/admin';

function onRatePath(): boolean {
  return location.pathname.replace(/\/+$/, '') === RATE_PATH;
}

/// Move between the two without a page load, keeping history honest so Back
/// leaves a session rather than trapping someone in it.
function go(path: string, replace = false): void {
  if (location.pathname !== path) {
    history[replace ? 'replaceState' : 'pushState']({}, '', path);
  }
  void boot();
}

async function boot(): Promise<void> {
  if (onRatePath()) {
    await enterStudy();
    return;
  }
  const path = location.pathname.replace(/\/+$/, '');
  if (path === BOARD_PATH) {
    await renderBoard(root, { onBack: () => go('/'), onStart: () => go(RATE_PATH) });
    return;
  }
  if (path === ADMIN_PATH) {
    await showAdmin(root, () => go('/'));
    return;
  }
  // The landing page. Opening squintly used to run straight into a session —
  // for a returning observer, literally into the next trial — so the decision
  // to take part was made on the one screen with nothing on it to decide from.
  await renderLanding(root, {
    onStart: () => go(RATE_PATH),
    onSignIn: () => openSignInModal(),
    onSignOut: async () => {
      await signOut();
      await boot();
    },
    onCalibrate: () => openCalibration(),
    onBoard: () => go(BOARD_PATH),
    onAdmin: () => go(ADMIN_PATH),
  });
}

// Back/forward moves between the front page and the session like any other
// site, rather than leaving whatever screen happened to be mounted.
window.addEventListener('popstate', () => void boot());

/// From the landing page into the study.
///
/// The instructions gate sits here rather than in `boot` so that arriving at
/// the front door is free — a person reading the page to decide whether to take
/// part has not agreed to a three-second hold yet.
async function enterStudy(): Promise<void> {
  const returning = hasOnboarded();
  await maybeShowInstructions(root, { returning });
  if (!returning) {
    await welcome();
    return;
  }
  // Everyone who onboarded before the chooser existed had a mode picked for
  // them by device class. Asking here rather than never is the whole reason
  // `hasChosenInputMode` is separate from "which mode are we in" — and it is a
  // one-off, because answering it is what makes it stop.
  if (!hasChosenInputMode()) await chooseInputMode(root);

  // Ask about last time before starting this time.
  //
  // This is the PRIMARY moment for a debrief, not the fallback: almost nobody
  // clicks End session, so a prompt that only fires there mostly never fires.
  // Here it lands on somebody who is already at their device and about to work,
  // which is also the only point where it cannot interrupt a sitting.
  //
  // Nothing is asked if there is no closed bout to ask about — a first-time
  // observer, a bout too short to have an impression of, or one already
  // debriefed. `maybeShowDebrief` calls `onDone` immediately in that case.
  const observerId = getObserverId();
  if (observerId) {
    await new Promise<void>((done) => {
      void maybeShowDebrief(root, observerId, { promptedAt: 'return', onDone: done });
    });
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
