// Squintly entrypoint. Routes between welcome → calibration → profile → trials.

import { createSession } from './api';
import { renderCalibration } from './calibration';
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
import { startTrials } from './trial';

const root = document.getElementById('app')!;

async function welcome(): Promise<void> {
  // Probe codec support before rendering — the result feeds the JXL banner.
  const support = await detectCodecs();
  const jxlHint = jxlEnableHint(support.supported);

  const banner = jxlHint
    ? `<p class="muted" style="background:#1a1f24;border:1px solid #2c3e50;padding:12px;border-radius:8px;line-height:1.4;">${jxlHint}</p>`
    : '';

  root.innerHTML = `
    <div class="screen center">
      <h1>Image Discrimination Study</h1>
      <p>You'll help train an open-source perceptual quality metric used by Wikipedia, Imageflow, and the JPEG XL ecosystem. We need ratings from real phones, in real lighting, at real viewing distances — that's the data the existing public datasets don't have.</p>
      <p>You'll see a series of images and rate how each one compares to its original. ~5 minutes; the more you do, the more it helps.</p>
      <p class="muted">No login, no personal info. Only screen and rating data are recorded.</p>
      ${banner}
      <button id="begin" class="primary">Begin</button>
      <p class="muted">Best on phones, but any browser works.</p>
    </div>
  `;
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
