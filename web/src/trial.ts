// Trial loop. Single-stimulus 4-tier ACR by default; pair trials use 3-button
// "A closer / tie / B closer". The reference is always reachable — by button,
// by press-and-hold, by keyboard, or (in `hold` mode) as the resting state.

import {
  listLeaderboard,
  whoami,
  signOut,
  listStudies,
  nextTrial,
  recordResponse,
  type LeaderboardRow,
  type TrialPayload,
} from './api';
import { captureTrial, getObserverId, loadCalibration, loadStudyId, saveStudyId } from './conditions';
import { showInstructions } from './instructions';
import { notify } from './notify';
import { openSignInModal } from './auth-modal';
import { showAdmin } from './admin';
import {
  HoldStack,
  buttonForKey,
  diffButtons,
  holdIdFor,
  restingView,
  viewForPress,
} from './hold-stack';
import {
  INPUT_MODE_LABELS,
  INPUT_MODE_LABELS_LONG,
  availableInputModes,
  type InputMode,
  isInputMode,
  inputModeHint,
  loadInputMode,
  saveInputMode,

} from './input-mode';

type View = 'a' | 'b' | 'ref';

/// Whole-number magnification only, every step available.
///
/// Was `[1, 2, 4, 8]`. Integer factors are non-negotiable (a fractional one
/// sizes some source pixels 2 device px and others 3 — fabricated structure in
/// a study about which structure is real), but there is no reason to skip 3, 5,
/// 6 and 7: they are just as exact. The wheel snaps onto these stops rather
/// than scaling continuously.
const ZOOM_LADDER = [1, 2, 3, 4, 5, 6, 7, 8];

/// How long an observer may hold variants up before the UI says a tie is a
/// real answer.
///
/// Cumulative time with something pressed, not wall-clock on the trial — see
/// `heldMsNow`. Nine seconds is a lot of active flicking: a pair anyone can
/// separate is settled in one or two looks, so this fires on the tail, not on
/// ordinary care. Tunable in one place because the right value is an empirical
/// question — `cant_tell_hint_ms` in the export is what answers it.
const CANT_TELL_HINT_AFTER_HELD_MS = 9_000;

/// How far two contacts must separate before a second finger counts as a pinch
/// rather than as a second press.
///
/// Two fingers on the glass are ambiguous: sizing the picture, or holding one
/// half while tapping the other to compare. Committing on the second
/// `pointerdown` resolved that ambiguity the wrong way and silently disabled
/// two-finger comparison. Distance is what actually distinguishes them, and it
/// costs a few pixels of travel to find out.
const PINCH_COMMIT_CSS = 12;

/// Whether the observer has dismissed the how-to pill.
///
/// localStorage, deliberately not a response column or a server field: it is a
/// preference about chrome, it tells us nothing about a judgement, and putting
/// it in the database would mean a migration and an extra write per session for
/// a value nothing downstream reads. Losing it on a new device costs one tap.
const HINT_DISMISSED_KEY = 'squintly_hint_dismissed';

function hintDismissed(): boolean {
  try {
    return localStorage.getItem(HINT_DISMISSED_KEY) === '1';
  } catch {
    return false;
  }
}

function dismissHint(): void {
  try {
    localStorage.setItem(HINT_DISMISSED_KEY, '1');
  } catch {
    /* private mode: the tip simply comes back next load */
  }
}

interface TrialState {
  shownAt: number;
  revealCount: number;
  revealMsTotal: number;
  zoomUsed: boolean;
  /// Distinct drag gestures, and total travel. With 1:1 display mandatory, a
  /// stimulus larger than the screen is only partly visible at any moment, so
  /// whether the observer explored it is part of the response.
  panCount: number;
  panDistanceCss: number;
  /// Magnification at response time. Integer, >= 1 (never a downscale).
  /// A judgement at 4x is a different observation from one at 1x — the visual
  /// angle an artefact subtends differs, which is what this study conditions on.
  zoomFactor: number;
  keyboardUsed: boolean;
  /// How many times the view changed. A pair flipped between six times is near
  /// the observer's discrimination threshold; one answered after a single look
  /// is not. BT treats both answers identically, so this is the only record
  /// that the pair was hard.
  switchCount: number;
  /// Milliseconds each variant was actually on screen. `reveal_ms_total` only
  /// ever measured the reference, which under `hold`/`buttons` is the resting
  /// view and therefore says nothing about effort.
  msOnView: { a: number; b: number; ref: number };
  /// Time from render to the judged image being painted. Kept separable from
  /// `dwell_ms`: waiting for a decode is not deliberation.
  uiReadyMs: number | null;
  /// When the UI first suggested "can't tell", in ms from the trial appearing.
  /// `null` means it never did. Recorded because it is a nudge toward one
  /// specific answer on exactly the hardest trials — see migration 0021.
  cantTellHintMs: number | null;
}

export interface TrialController {
  start(): Promise<void>;
  end(): void;
}

export interface TrialHooks {
  /// The observer chose a different study. A session belongs to exactly one
  /// study, so this cannot be applied in place — the caller ends this session
  /// and starts a fresh one under the new choice.
  onSwitchStudy: () => void;
  /// Re-run screen-size calibration, then resume.
  onRecalibrate: () => void;
}

export function startTrials(
  root: HTMLElement,
  sessionId: string,
  hooks: TrialHooks,
): TrialController {
  const { onSwitchStudy, onRecalibrate } = hooks;
  /// The trial currently on screen, so the menu can re-render it after a
  /// settings change without re-fetching.
  let currentTrial: TrialPayload | null = null;
  /// The trial just answered, kept so a misclick can be taken back.
  ///
  /// Only the immediately-previous one: reaching further back would let an
  /// observer revise in light of trials they saw afterwards, which is a
  /// different and much less innocent thing than fixing a stray tap. The server
  /// enforces the same rule (`record_response` refuses a revision once a later
  /// response exists), so this is not the only line of defence.
  let lastAnswered: { trial: TrialPayload; choice: string } | null = null;
  /// Opens the keyboard cheatsheet for the trial on screen.
  let showKeyHelp: (() => void) | null = null;
  let aborted = false;
  let trialCount = 0;
  /// Magnification persists across trials in a session — see the note where it
  /// is applied. Recorded per response, so persistence costs no fidelity.
  let zoomFactor = 1;
  let inputMode: InputMode = loadInputMode();
  /// Torn down before each render; a stale listener would drive the previous
  /// trial's closure and submit against a trial that is no longer on screen.
  let detachKeys: (() => void) | null = null;
  /// Same reasoning for the "can't tell" ticker: left running it would keep
  /// reading the previous trial's state and could mark a hint on a trial the
  /// observer has already left.
  let stopNudge: (() => void) | null = null;
  /// Lifetime comparisons and the lap length, both from the server.
  ///
  /// The threshold is `crowd_bt::MIN_OBS_FOR_ETA` — the point at which this
  /// observer's reliability can be estimated at all, and therefore the point at
  /// which their answers become weighable rather than merely stored. That is
  /// why the bar counts what it counts: it is showing a real boundary, not a
  /// made-up one, which is the only kind of progress bar worth drawing.
  let lapProgress: { done: number; per: number } | null = null;

  const calib = loadCalibration();

  /// Which study this session belongs to, in two words, for the corner of the
  /// screen. Resolved once and cached: an observer who has switched studies
  /// mid-run should be able to see which one they are in without opening the
  /// menu, and the id (`ssim2-nonphoto`) is not a thing to read at a glance.
  let studyShortName: string | null = null;
  void (async () => {
    try {
      const chosen = loadStudyId();
      const studies = await listStudies();
      const s = studies.find((x) => x.id === chosen) ?? studies[0];
      studyShortName = s?.short_name ?? null;
      const badge = root.querySelector<HTMLElement>('#study-badge');
      if (badge && studyShortName) badge.textContent = studyShortName;
    } catch {
      /* the corner label is not worth failing a trial over */
    }
  })();

  const fetchAndRender = async () => {
    if (aborted) return;
    renderLoading();
    let trial: TrialPayload;
    try {
      trial = await nextTrial(sessionId);
    } catch (e) {
      root.innerHTML = `<div class="screen center"><h1>No trials available</h1><p class="muted">${
        (e as Error).message
      }</p></div>`;
      return;
    }
    if (aborted) return;
    renderTrial(trial);
  };

  /// Shown while the next trial is being chosen. The server picks it with ASAP
  /// active sampling over the answers so far, so this round trip cannot be
  /// prefetched away (see the note on preloading below) — but it can at least
  /// say what it is doing instead of leaving a blank frame.
  const renderLoading = () => {
    root.innerHTML = `
      <div class="screen center trial-loading" data-screen="trial-loading">
        <div class="spinner" role="status" aria-label="Loading the next trial"></div>
        <p class="muted">Choosing the next comparison…</p>
      </div>
    `;
  };

  const renderTrial = (trial: TrialPayload) => {
    currentTrial = trial;
    detachKeys?.();
    detachKeys = null;
    stopNudge?.();
    stopNudge = null;

    const renderedAt = performance.now();
    const state: TrialState = {
      shownAt: 0,
      revealCount: 0,
      revealMsTotal: 0,
      zoomUsed: false,
      panCount: 0,
      panDistanceCss: 0,
      zoomFactor,
      keyboardUsed: false,
      switchCount: 0,
      msOnView: { a: 0, b: 0, ref: 0 },
      cantTellHintMs: null,
      uiReadyMs: null,
    };

    const isPair = trial.kind === 'pair';
    const corpus = trial.source_corpus ?? 'unknown';
    const licId = trial.source_license_id;
    const licLabel = trial.source_license_label;
    const views: View[] = isPair ? ['a', 'b', 'ref'] : ['a', 'ref'];
    const srcFor = (v: View) =>
      v === 'ref' ? trial.source_url : v === 'a' ? trial.a.url : trial.b!.url;

    // A one-option select is a control that cannot do anything. On touch there
    // is exactly one mode, so the picker is simply absent there.
    // Touch has one mode, no keyboard sheet and no zoom stepper, but still has
    // the least room — so the labels that remain get shorter rather than the
    // row getting taller.
    const compact = availableInputModes().length === 1;
    const modePicker = availableInputModes().length > 1
      ? `<label class="mode-picker">
           <span class="sr-only">Interaction mode</span>
           <select id="input-mode" aria-label="Interaction mode">
             ${availableInputModes().map(
               (m) =>
                 `<option value="${m}"${m === inputMode ? ' selected' : ''}>${INPUT_MODE_LABELS[m]}</option>`,
             ).join('')}
           </select>
         </label>`
      : '';

    root.innerHTML = `
      <div class="trial" data-trial-id="${trial.trial_id}" data-input-mode="${inputMode}">
        <div class="lap" id="lap" hidden>
          <div class="lap-fill" id="lap-fill"></div>
        </div>
        <div class="progress">
          <span class="study-badge" id="study-badge">${escapeHtml(studyShortName ?? '')}</span>
          <span>Trial ${trialCount + 1}</span>
          <span class="trial-license" data-corpus="${escapeAttr(corpus)}" data-license-id="${escapeAttr(licId)}" title="${escapeAttr(`${trial.source_filename ?? corpus} · ${corpus} · ${licLabel}`)}">${
            trial.source_group ? `<span class="src-group">${escapeHtml(trial.source_group)}</span> ` : ''
          }${escapeHtml(trial.source_label ?? corpus)}</span>
          <button class="menu-btn" id="menu" aria-label="Menu" title="Menu"></button>
        </div>
        <div class="stage" id="stage" data-view="${restingView(inputMode)}">
          <div class="edge edge-left" aria-hidden="true"></div>
          <div class="edge edge-right" aria-hidden="true"></div>
          <div class="edge edge-top" aria-hidden="true"></div>
          <div class="viewport is-loading" id="viewport">
            ${views
              .map(
                (v) =>
                  `<img class="layer" data-layer="${v}" alt="" decoding="async" fetchpriority="high" />`,
              )
              .join('')}
            <div class="viewport-status" id="vp-status">
              <div class="spinner" role="status" aria-label="Loading images"></div>
            </div>
          </div>
        </div>
        <div class="trial-controls">
          <div class="view-switch" id="view-switch" role="group" aria-label="Which image">
            ${
              isPair
                ? `<button data-view="a" class="on">A</button>
                   <button data-view="b">B</button>
                   <button data-view="ref">${compact ? 'Orig' : 'Original'}</button>`
                : `<button data-view="a" class="on">${compact ? 'Comp' : 'Compressed'}</button>
                   <button data-view="ref">${compact ? 'Orig' : 'Original'}</button>`
            }
          </div>
          <div class="zoom-switch" id="zoom-switch" role="group" aria-label="Magnification">
            <button data-zoom-step="-1" aria-label="Magnify less">−</button>
            <output id="zoom-readout" aria-live="polite">1×</output>
            <button data-zoom-step="1" aria-label="Magnify more">+</button>
          </div>
          ${modePicker}
          <button class="undo-btn" id="undo-btn" aria-label="Take back the previous answer"
                  title="Take back the previous answer (u)"${lastAnswered ? '' : ' hidden'}>↶${compact ? '' : ' undo'}</button>
          <button class="keys-btn" id="info-btn" aria-label="Image identifiers" title="Which images am I looking at? (i)">i</button>
          <button class="keys-btn" id="keys-btn" aria-label="Keyboard shortcuts" title="Keyboard shortcuts (?)">⌨</button>
        </div>
        <div class="reveal-hint" id="hint" hidden>
          <span id="hint-text"></span>
          <button class="hint-dismiss" id="hint-dismiss" aria-label="Hide this tip">&times;</button>
        </div>
        <p class="gate-hint" id="gate-hint" hidden></p>
        <div id="panel"></div>
      </div>
    `;
    const viewport = root.querySelector<HTMLDivElement>('#viewport')!;
    const stage = root.querySelector<HTMLDivElement>('#stage')!;
    const panel = root.querySelector<HTMLDivElement>('#panel')!;
    const hint = root.querySelector<HTMLDivElement>('#hint')!;
    const hintText = root.querySelector<HTMLSpanElement>('#hint-text')!;
    const status = root.querySelector<HTMLDivElement>('#vp-status')!;
    const gateHint = root.querySelector<HTMLParagraphElement>('#gate-hint')!;
    root.querySelector<HTMLButtonElement>('#menu')!.addEventListener('click', () => openMenu());

    // ---- every variant is loaded up front -------------------------------
    //
    // One <img> whose `src` was rewritten on each switch meant every A→B→
    // original flip re-decoded: a blank frame, a re-layout from
    // naturalWidth 0, and a visible flash. That is what made comparing feel
    // clunky, and it is worst exactly when it matters most — flicking back
    // and forth to find a difference.
    //
    // All variants are now separate layers, fetched in parallel the moment the
    // trial arrives and only toggled for visibility. Switching costs nothing
    // after that, which is the whole point: A/B comparison is a
    // same-place-different-picture task, and any latency between the two
    // pictures is latency the observer has to hold in memory.
    //
    // The *next* trial is deliberately not prefetched. The server chooses it
    // with ASAP active sampling over the responses so far, so fetching it
    // early would pick the next stimulus without the answer being given —
    // trading measurement efficiency for a saved round trip.
    const layers = {} as Record<View, HTMLImageElement>;
    for (const v of views) {
      layers[v] = viewport.querySelector<HTMLImageElement>(`img[data-layer="${v}"]`)!;
    }

    const dpr = window.devicePixelRatio ?? 1;
    const pan = { x: 0, y: 0 }; // CSS px offset from the centred crop
    const panLimit = { x: 0, y: 0 };
    // In `hold` mode the reference is what you see at rest; in `tap` mode the
    // encoding is, and the reference is a peek.
    const resting: View = restingView(inputMode);
    let currentSrc: View = resting;
    // Which encoding the observer is judging, independent of whether they are
    // momentarily looking at the reference. Kept separate so flipping to the
    // original and back cannot lose their place in an A/B comparison.
    let choiceSrc: 'a' | 'b' = 'a';
    // Magnification. INTEGER factors only, painted nearest-neighbour
    // (`image-rendering: pixelated`), so one image pixel becomes an exact N×N
    // block of device pixels. Interpolating would invent values the codec
    // never produced, and a fractional factor would size some source pixels
    // 2 device px and others 3 — fabricated structure, in a study whose whole
    // subject is which structure is real. Persisted across trials within the
    // session: re-zooming every trial is hostile, and the factor is recorded
    // per response anyway.
    let zoom = zoomFactor;
    let submitted = false;
    /// Which variants the observer has actually looked at.
    ///
    /// "Which is closer to the original" is not answerable from one arm, so the
    /// response panel stays locked until both have been on screen. Under `hold`
    /// and `buttons` the resting view is the reference, so this forces a
    /// deliberate look at each side rather than a reflex answer; under `tap` A
    /// is already up, so only B has to be sought out.
    const seen = new Set<View>();

    // Function DECLARATIONS, not const arrows: `showView` runs during setup and
    // calls `refreshGate`, which calls these. As `const` they sat in the
    // temporal dead zone at that point and every render threw — the trial screen
    // simply never appeared.
    /// Which arms must have been looked at before an answer is allowed.
    function requiredViews(): View[] {
      return isPair ? ['a', 'b'] : ['a'];
    }
    function allSeen(): boolean {
      return requiredViews().every((v) => seen.has(v));
    }

    const clampPan = () => {
      pan.x = Math.max(-panLimit.x, Math.min(panLimit.x, pan.x));
      pan.y = Math.max(-panLimit.y, Math.min(panLimit.y, pan.y));
    };
    /// Pan applies to every layer, so a switch lands on the same region — you
    /// are comparing the same part of the picture, not two different parts.
    ///
    /// The leading -50% is the centring: layers are anchored at the frame's
    /// centre point (`left/top: 50%`) because `margin: auto` silently stops
    /// centring once the box overflows its container. See the note in
    /// `style.css`.
    const applyPan = () => {
      const t = `translate(-50%, -50%) translate(${pan.x}px, ${pan.y}px)`;
      for (const v of views) layers[v].style.transform = t;
    };

    const sizeLayer = (el: HTMLImageElement) => {
      if (el.naturalWidth === 0) return;
      // 1:1 device pixels, mandatory. The stimulus is rendered at exactly one
      // image pixel per device pixel (CSS size = intrinsic / dpr), and NEVER
      // smaller. Anything larger than the viewport is explored by dragging.
      //
      // This used to `Math.min(1, …)` down to whatever fitted, which silently
      // resampled the stimulus in the browser — the observer was then rating
      // the *browser's* downscale rather than the encode, and the artefacts
      // under test get averaged away exactly where the study cares most
      // (high-DPR phones, large sources). Zooming in is fine; going below 1:1
      // is not.
      el.style.width = `${(el.naturalWidth * zoom) / dpr}px`;
      el.style.height = `${(el.naturalHeight * zoom) / dpr}px`;
      el.style.maxWidth = 'none'; // the stylesheet's max-width:100% would re-shrink it
      el.style.maxHeight = 'none';
    };

    const recomputePanLimits = () => {
      const el = layers[currentSrc];
      const rect = viewport.getBoundingClientRect();
      const w = parseFloat(el.style.width || '0');
      const h = parseFloat(el.style.height || '0');
      // A layer that has not decoded yet has no size. Recomputing from zero
      // would collapse the limits and `clampPan` would snap the observer back
      // to the centre — losing their place on a swap to a still-loading
      // variant, which is exactly what carrying the pan across views exists to
      // prevent. Keep the existing limits; `markReady` recomputes for real.
      if (!(w > 0) || !(h > 0)) return;
      // Centred on the frame's midpoint, so it travels half the overflow each way.
      panLimit.x = Math.max(0, (w - rect.width) / 2);
      panLimit.y = Math.max(0, (h - rect.height) / 2);
      clampPan();
      applyPan();
      viewport.classList.toggle('pannable', isPannable());
      // The hint advertises dragging, so it is a function of the pan limits and
      // has to be refreshed wherever they are. Layers size asynchronously on
      // decode, so limits are 0 at first paint and only become real here.
      updateHint();
    };

    const isPannable = () => panLimit.x > 0.5 || panLimit.y > 0.5;

    const viewSwitch = root.querySelector<HTMLDivElement>('#view-switch')!;
    const zoomSwitch = root.querySelector<HTMLDivElement>('#zoom-switch')!;
    const zoomReadout = root.querySelector<HTMLOutputElement>('#zoom-readout')!;

    const markActive = (host: HTMLElement, attr: string, value: string) => {
      host.querySelectorAll<HTMLButtonElement>('button').forEach((b) => {
        b.classList.toggle('on', b.dataset[attr] === value);
      });
    };

    function updateHint() {
      // Dismissed for good — the gesture is learned in a trial or two, and after
      // that the pill is a permanent band of text beside the picture on the
      // screen with the least room for one.
      if (hintDismissed()) {
        hint.hidden = true;
        return;
      }
      const bits: string[] = [];
      // "drag to explore" is never in the gate hint, so it survives: an
      // oversized stimulus has to advertise panning even while the gate is
      // still closed, or the observer cannot tell there is more picture.
      if (isPannable()) bits.push('drag to explore');
      // The GESTURE line is the duplicate. While the gate is closed
      // `#gate-hint` is already saying "press and hold the left and right
      // half" — the same sentence, one line apart, on a phone. The gate's
      // version wins there because it is the actionable one (it says why the
      // panel is locked and goes away once you have looked); this one returns
      // when the gate opens.
      if (gateHint.hidden) bits.push(inputModeHint(inputMode, isPair));
      hintText.textContent = bits.join(' · ');
      hint.hidden = bits.length === 0;
    }
    root.querySelector<HTMLButtonElement>('#hint-dismiss')!.addEventListener('click', () => {
      dismissHint();
      updateHint();
    });

    // ---- reveal accounting ----------------------------------------------
    //
    // "Reveal" is time the *reference* was on screen, in both modes. Under
    // `tap` that is a deliberate peek; under `hold` it is the resting state
    // and will dominate the trial. Both are recorded the same way and the mode
    // is stored alongside, so an analyst can tell them apart — see migration
    // 0017. Inferring the mode from the magnitude of this number afterwards
    // would be guessing.
    /// When the currently-shown view went up. Every view is timed, not just the
    /// reference — see `msOnView`.
    let viewShownAt: number | null = null;
    const closeViewAccounting = (now: number) => {
      if (viewShownAt !== null) {
        const held = now - viewShownAt;
        state.msOnView[currentSrc] += held;
        // Kept in step so the existing column keeps its meaning.
        if (currentSrc === 'ref') state.revealMsTotal += held;
        viewShownAt = null;
      }
    };

    /// Show a given variant. Zero-cost after load: everything is already
    /// decoded, so this only flips which layer is visible.
    const showView = (which: View) => {
      if (which === 'b' && !isPair) return;
      const now = performance.now();
      if (which !== currentSrc) {
        closeViewAccounting(now);
        // Only count a switch once the trial is actually up; the initial
        // render is not something the observer did.
        if (state.shownAt > 0) state.switchCount += 1;
        if (which === 'ref') state.revealCount += 1;
        viewShownAt = now;
      } else if (viewShownAt === null) {
        viewShownAt = now;
      }
      currentSrc = which;
      seen.add(which);
      viewport.dataset.view = which;
      // The letterbox tiling disappears the moment the stimulus covers the
      // frame — which is most of the time once someone magnifies — so the edge
      // frame carries the same signal somewhere it cannot be covered.
      stage.dataset.view = which;
      // Marks "you are looking at the original" — accents the hint pill, and is
      // what the e2e suite reads to tell the two states apart. Correct in both
      // modes: under `hold` the reference is the resting view, so it is on
      // until you press a button.
      root.querySelector('.trial')?.classList.toggle('revealing', which === 'ref');
      for (const v of views) {
        const el = layers[v];
        const on = v === which;
        el.classList.toggle('shown', on);
        // `#stimulus` is the contract for "the image the observer is looking
        // at" — conditions capture, grading geometry and the e2e suite all
        // read it — so it moves with the visible layer.
        if (on) el.id = 'stimulus';
        else el.removeAttribute('id');
      }
      recomputePanLimits();
      markActive(viewSwitch, 'view', which);
      refreshGate();
    };

    // ---- load / decode tracking -----------------------------------------
    let pending = views.length;
    /// Nothing is interactive until EVERY variant is paint-ready.
    ///
    /// This used to unlock as soon as the *judged* layer arrived, which left
    /// two ways to see an empty frame. Pressing B (or the view switch) while B
    /// was still on the wire showed nothing at all — a real source is ~9.5 MB,
    /// so that is not a flash but a blank viewport. And `load` only means
    /// "decodable", not decoded: a `visibility: hidden` layer is never painted,
    /// so the first flip had to decode and rasterise on the spot, costing a
    /// frame. `decode()` exists for exactly this — it resolves when the bitmap
    /// is ready to paint without a hitch.
    ///
    /// A switch that flashes is not merely untidy here: it injects a visual
    /// transient between the two pictures being compared, at the instant of
    /// comparison.
    // Idempotent per layer. A variant that finishes between the `load` listener
    // being attached and the already-cached sweep below satisfies BOTH paths,
    // so without this the same layer decrements `pending` twice and the trial
    // reports itself ready while another variant is still on the wire —
    // exactly the "clickable before B has arrived" bug this gate exists to
    // close, reintroduced through the back door.
    const settled = new Set<View>();
    const markReady = (v: View) => {
      if (settled.has(v)) return;
      settled.add(v);
      sizeLayer(layers[v]);
      pending -= 1;
      if (pending > 0) return;

      viewport.classList.add('all-ready');
      viewport.classList.remove('is-loading');
      status.hidden = true;
      // Before the panel is usable, not after: the observer should never be
      // offered a judgement on a stimulus too small to judge.
      ensureCovers();
      refreshGate();
      recomputePanLimits();
      if (state.shownAt === 0) {
        state.shownAt = performance.now();
        state.uiReadyMs = Math.round(state.shownAt - renderedAt);
      }
    };

    for (const v of views) {
      const el = layers[v];
      const ready = () => {
        // `decode()` rejects if the element is torn down mid-flight (the
        // observer answered), which is not an error worth surfacing.
        el.decode()
          .catch(() => {})
          .then(() => markReady(v));
      };
      el.addEventListener('load', ready);
      el.addEventListener('error', () => {
        if (settled.has(v)) return;
        settled.add(v);
        pending -= 1;
        status.innerHTML = `<p class="muted">An image failed to load.</p>`;
        status.hidden = false;
      });
      el.src = srcFor(v);
    }
    showView(resting);
    paintLap();
    // Anything already in cache resolves before the listener attached above.
    for (const v of views) {
      const el = layers[v];
      if (el.complete && el.naturalWidth > 0) {
        el.decode()
          .catch(() => {})
          .then(() => markReady(v));
      }
    }

    viewSwitch.querySelectorAll<HTMLButtonElement>('button').forEach((b) => {
      b.addEventListener('click', () => {
        const v = b.dataset.view as View | undefined;
        if (!v) return;
        // An explicit pick moves the resting view; a transient hold must not.
        // `showView` used to set `choiceSrc` itself, so peeking at B with the
        // right button silently redefined "resting" as B and releasing left it
        // there — the hold never came back.
        if (v !== 'ref') choiceSrc = v;
        showView(v);
      });
    });

    const applyZoom = (next: number) => {
      if (next === zoom || !Number.isFinite(next) || next < 1) return;
      // Keep whatever is centred, centred: at 1×→2× the same feature sits
      // twice as far from centre, so the offset scales with it.
      const ratio = next / zoom;
      pan.x *= ratio;
      pan.y *= ratio;
      zoom = next;
      zoomFactor = next;
      state.zoomUsed = true;
      state.zoomFactor = next;
      zoomReadout.textContent = `${next}×`;
      zoomSwitch.dataset.zoom = String(next);
      // Every layer is resized, not just the visible one — otherwise switching
      // after a zoom would jump between two magnifications.
      for (const v of views) sizeLayer(layers[v]);
      recomputePanLimits();
    };
    const stepZoom = (dir: 1 | -1) => {
      const i = ZOOM_LADDER.indexOf(zoom);
      const next = ZOOM_LADDER[Math.max(0, Math.min(ZOOM_LADDER.length - 1, (i < 0 ? 0 : i) + dir))];
      applyZoom(next);
    };

    zoomSwitch.querySelectorAll<HTMLButtonElement>('button').forEach((b) => {
      b.addEventListener('click', () => stepZoom(Number(b.dataset.zoomStep) as 1 | -1));
    });
    zoomReadout.textContent = `${zoom}×`;
    zoomSwitch.dataset.zoom = String(zoom);

    root.querySelector<HTMLSelectElement>('#input-mode')?.addEventListener('change', (e) => {
      const v = (e.target as HTMLSelectElement).value;
      if (!isInputMode(v)) return;
      inputMode = v;
      saveInputMode(v);
      // Re-render so the resting view and pointer bindings match the new mode.
      // Cheap: every variant is already in cache.
      renderTrial(trial);
    });

    // ---- pointer --------------------------------------------------------
    //
    // Drag pans; a press that doesn't move is a tap/hold. Movement past a small
    // threshold separates them, otherwise every pan would also fire a reveal or
    // flip A/B.
    const DRAG_THRESHOLD_CSS = 6;

    interface Held {
      startX: number;
      startY: number;
      lastX: number;
      lastY: number;
      downAt: number;
      moved: boolean;
    }

    // EVERY active pointer is tracked, not just the first.
    //
    // The old model kept a single `pointerId` and ignored any pointer that
    // arrived while one was down. Two consequences on a touchscreen, both
    // reported: putting one finger on the left and a second on the right, then
    // lifting the first, snapped to the original — the release ran the
    // end-of-gesture handler even though a finger was still down, and the
    // second finger's own release was then ignored because its id no longer
    // matched. And pinch could not exist at all, because the second finger was
    // never admitted.
    const held = new Map<number, Held>();
    let gesture: 'none' | 'pan' | 'pinch' = 'none';
    let pinchStartDist = 0;
    let pinchStartZoom = 1;

    /// Which half of the frame a press landed in.
    const pressedHalf = (x: number): 'left' | 'right' => {
      const r = viewport.getBoundingClientRect();
      return x < r.left + r.width / 2 ? 'left' : 'right';
    };

    /// Every held input — mouse buttons, touches and keys alike — in one stack.
    /// The most recent still-held press decides what is on screen; releasing it
    /// falls back to the next one down rather than to the resting view. See
    /// `hold-stack.ts` for the table and the ordering cases.
    const holds = new HoldStack();

    /// What is on screen when nothing is held. Under `tap` that is the variant
    /// the observer last chose with the view switch, not a fixed side.
    const restingNow = (): View => (inputMode === 'tap' ? choiceSrc : 'ref');

    /// Apply whatever the stack currently implies.
    ///
    /// Suppressed mid-pinch: sizing the picture is not a comparison gesture, so
    /// the second contact must not swap the variant under it. The holds are
    /// still tracked, so lifting one finger falls back to the other rather than
    /// to the resting view — clearing them here is what reintroduced "two
    /// fingers, lift one, the original appears".
    const applyHolds = () => {
      // Only a committed pinch suppresses the view — flipping the variant while
      // someone is sizing the picture would change it under them. Two contacts
      // alone are not a pinch: see the pointerdown handler.
      if (gesture === 'pinch') return;
      showView(holds.resolve(restingNow()));
    };

    /// Mouse button state, reconciled from the `buttons` bitmask on every
    /// pointer event — a second button press fires no `pointerdown`, so
    /// down/up events alone cannot see it. See `diffButtons`.
    let lastButtons = 0;
    const syncButtons = (e: PointerEvent) => {
      if (e.pointerType === 'mouse') {
        const { pressed, released } = diffButtons(lastButtons, e.buttons, e.pointerId);
        lastButtons = e.buttons;
        for (const id of released) holds.release(id);
        for (const { id, button } of pressed) {
          holds.press(
            id,
            viewForPress(button, {
              mode: inputMode,
              isPair,
              half: pressedHalf(e.clientX),
            }),
          );
        }
        if (pressed.length || released.length) applyHolds();
        return;
      }
      // Touch: one contact, one hold, keyed by pointer.
      //
      // Only the events that actually END a contact release it. This was
      // `else { holds.release(id) }`, and `pointermove` routes through here —
      // so a single pixel of movement released the hold and the variant snapped
      // back to the resting view. Under `hold`, the only touch mode, that is
      // the whole gesture: a thumb resting on the glass is never perfectly
      // still, so the comparison collapsed almost immediately.
      //
      // A moving contact is still a contact. Panning is driven separately from
      // `held`, so nothing here needs to know about the drag.
      const id = holdIdFor(e.pointerType, e.pointerId, e.button);
      if (e.type === 'pointerdown') {
        holds.press(
          id,
          viewForPress('touch', {
            mode: inputMode,
            isPair,
            half: pressedHalf(e.clientX),
          }),
        );
      } else if (e.type === 'pointerup' || e.type === 'pointercancel') {
        holds.release(id);
      } else {
        return;
      }
      applyHolds();
    };

    const twoPointerDistance = (): number => {
      const [a, b] = [...held.values()];
      if (!a || !b) return 0;
      return Math.hypot(a.lastX - b.lastX, a.lastY - b.lastY);
    };

    /// Smallest whole factor at which the image covers the frame in BOTH
    /// dimensions.
    ///
    /// An S-bucket source is 240px, which at 1:1 on a DPR-3 phone is about 80
    /// CSS px — a postage stamp with acres of black around it, and no way to
    /// see the artefacts being rated. Magnifying to cover is the correct fix
    /// and the *only* one available: the display rule forbids going below 1:1
    /// because that resamples the encode, but going above it at integer
    /// nearest-neighbour invents nothing — one source pixel becomes an exact
    /// N x N block.
    ///
    /// Capped by the ladder, so a very small source may still not fill the
    /// frame; that is better than a fractional factor.
    const coverFactor = (): number => {
      const el = layers[currentSrc];
      if (!el.naturalWidth || !el.naturalHeight) return 1;
      const r = viewport.getBoundingClientRect();
      if (r.width <= 0 || r.height <= 0) return 1;
      for (const z of ZOOM_LADDER) {
        if ((el.naturalWidth * z) / dpr >= r.width && (el.naturalHeight * z) / dpr >= r.height) {
          return z;
        }
      }
      return ZOOM_LADDER[ZOOM_LADDER.length - 1];
    };

    /// Raise magnification if the stimulus does not fill the frame.
    ///
    /// Only ever raises. Magnification persists across trials on purpose, so a
    /// deliberate 4x must survive a small source — this tops it up when the
    /// carried-over factor leaves the picture undersized, and leaves it alone
    /// otherwise.
    const ensureCovers = () => {
      const want = coverFactor();
      if (want > zoom) applyZoom(want);
    };


    // A long press over an image raises the callout/context menu on both mobile
    // and desktop, which would interrupt the hold exactly when it is the
    // primary gesture.
    viewport.addEventListener('contextmenu', (e) => {
      // The right button means B in every mode, so the context menu must never
      // interrupt it. It is also the only event some browsers deliver for a
      // right press while another button is held, so reconcile from it.
      e.preventDefault();
      if ('buttons' in e) syncButtons(e as PointerEvent);
    });

    // Suppress the native long-press gesture on touch.
    //
    // Belt-and-braces alongside `touch-action: none` and the `contextmenu`
    // handler above: on Android the long-press recogniser can fire
    // `pointercancel`, which is not cancellable, and `endPointer` is bound to
    // it — correctly, since a genuinely cancelled pointer must not leave a
    // stuck hold. Preventing the default action of `touchstart` stops the
    // recogniser before it starts. Pointer events are generated independently
    // of it, so nothing this code listens to is lost, and `passive: false` is
    // required or the preventDefault is ignored.
    //
    // This is NOT what caused holds to collapse mid-press — that was
    // `syncButtons` releasing a touch hold on `pointermove`, see there. Kept
    // because the callout is a real hazard for a press-and-hold UI, but it
    // fixed nothing on its own.
    viewport.addEventListener(
      'touchstart',
      (e) => {
        e.preventDefault();
      },
      { passive: false },
    );

    viewport.addEventListener('pointerdown', (e: PointerEvent) => {
      // A mouse fires `pointerdown` again for a second button on the SAME
      // pointer id. Restarting the drag record under it would zero an
      // in-progress pan, so only the first press of a pointer creates one.
      if (!held.has(e.pointerId)) {
        held.set(e.pointerId, {
          startX: e.clientX,
          startY: e.clientY,
          lastX: e.clientX,
          lastY: e.clientY,
          downAt: performance.now(),
          moved: false,
        });
      }
      syncButtons(e);
      try {
        viewport.setPointerCapture(e.pointerId);
      } catch {
        /* no capture (synthetic or already-released pointer); drag still works */
      }

      if (held.size >= 2) {
        // A second contact is a pinch CANDIDATE, not a pinch. Committing here
        // broke the ordering the hold stack exists to provide: holding the left
        // half and tapping the right did nothing at all, because `applyHolds`
        // refused to run while two fingers were down. But a second finger that
        // taps is a comparison gesture — "show me B while I keep A ready" — and
        // only a second finger that MOVES is sizing the picture.
        //
        // So record what a pinch would start from, keep applying holds, and let
        // `pointermove` commit to `pinch` if the distance actually changes.
        pinchStartDist = twoPointerDistance();
        pinchStartZoom = zoom;
        viewport.classList.remove('panning');
      } else {
        // FIRST contact: always a fresh gesture. Guarding this with
        // `if (gesture !== 'pinch')` stranded the state — once a pinch had been
        // committed nothing ever cleared it, and every later single-finger drag
        // was swallowed by the pinch branch instead of panning.
        gesture = 'none';
      }
      applyHolds();
    });

    viewport.addEventListener('pointermove', (e: PointerEvent) => {
      // Button changes arrive here, not as down/up, whenever another button is
      // already held.
      syncButtons(e);
      const h = held.get(e.pointerId);
      if (!h) return;
      const dx = e.clientX - h.lastX;
      const dy = e.clientY - h.lastY;
      h.lastX = e.clientX;
      h.lastY = e.clientY;
      if (Math.hypot(e.clientX - h.startX, e.clientY - h.startY) >= DRAG_THRESHOLD_CSS) {
        h.moved = true;
      }

      // Commit to a pinch only once the contacts have genuinely moved relative
      // to each other. Below this, two fingers are still a comparison gesture
      // and the hold stack keeps deciding what is on screen.
      if (gesture !== 'pinch' && held.size >= 2 && pinchStartDist > 0) {
        const d = twoPointerDistance();
        if (d > 0 && Math.abs(d - pinchStartDist) >= PINCH_COMMIT_CSS) {
          gesture = 'pinch';
        }
      }

      if (gesture === 'pinch') {
        // Pinch magnifies, snapping onto whole factors — a fractional one would
        // resample the stimulus. The ladder is walked by ratio, so the gesture
        // feels continuous even though the result never is.
        const d = twoPointerDistance();
        if (pinchStartDist > 0 && d > 0) {
          state.zoomUsed = true;
          const want = pinchStartZoom * (d / pinchStartDist);
          let nearest = ZOOM_LADDER[0];
          for (const z of ZOOM_LADDER) {
            if (Math.abs(z - want) < Math.abs(nearest - want)) nearest = z;
          }
          applyZoom(nearest);
        }
        return;
      }

      if (!h.moved) return;
      if (gesture !== 'pan') {
        gesture = 'pan';
        state.panCount += 1;
        if (isPannable()) viewport.classList.add('panning');
      }
      if (!isPannable()) return;
      pan.x += dx;
      pan.y += dy;
      state.panDistanceCss += Math.hypot(dx, dy);
      clampPan();
      applyPan();
    });

    const endPointer = (e: PointerEvent) => {
      const h = held.get(e.pointerId);
      if (!h) return;
      syncButtons(e);
      // `buttons` is the bitmask of what is STILL down. A mouse with another
      // button held is not finished, so the drag record stays.
      if (e.pointerType === 'mouse' && e.buttons !== 0) return;
      held.delete(e.pointerId);
      try {
        viewport.releasePointerCapture(e.pointerId);
      } catch {
        /* already released */
      }

      const wasPinch = gesture === 'pinch';
      if (held.size < 2 && wasPinch) gesture = 'none';

      // NO double-tap-to-fit. It read two quick presses in the same place as
      // "put the whole image back on screen" — but under `hold`, the only touch
      // mode, two quick presses in the same place is the COMPARISON: press a
      // half, release, press again. So the magnification kept resetting itself
      // mid-judgement, which is worse than not having the shortcut at all.
      // Pinch changes magnification on touch; the digits and the wheel do on a
      // mouse. A gesture cannot be reserved for a shortcut when the task has
      // already claimed it.

      if (held.size === 0) {
        gesture = 'none';
        lastButtons = 0;
        viewport.classList.remove('panning');
      }
      // Whatever is still held decides what stays on screen.
      applyHolds();
    };
    viewport.addEventListener('pointerup', endPointer);
    viewport.addEventListener('pointercancel', endPointer);

    // The wheel magnifies, snapping onto whole factors rather than scaling
    // continuously — a fractional factor would resample the stimulus, which is
    // the one thing this viewer refuses to do. Deltas accumulate so a
    // high-resolution trackpad does not fly from 1x to 8x in one flick, and a
    // notched mouse wheel still moves one stop per notch.
    //
    // `passive: false` so the page cannot scroll under the gesture; the trial
    // fills the viewport, so there is nothing to scroll anyway.
    let wheelAccum = 0;
    const WHEEL_STEP = 120; // one notch on a conventional mouse wheel
    viewport.addEventListener(
      'wheel',
      (e: WheelEvent) => {
        e.preventDefault();
        state.zoomUsed = true;
        wheelAccum += e.deltaY;
        while (wheelAccum <= -WHEEL_STEP) {
          wheelAccum += WHEEL_STEP;
          stepZoom(1);
        }
        while (wheelAccum >= WHEEL_STEP) {
          wheelAccum -= WHEEL_STEP;
          stepZoom(-1);
        }
      },
      { passive: false },
    );
    viewport.addEventListener('gesturestart', () => { state.zoomUsed = true; });

    // ---- response panel -------------------------------------------------
    if (isPair) {
      panel.innerHTML = `
        <div class="pair-panel">
          <button data-c="a"><span class="num">A</span><span>closer to original</span></button>
          <button data-c="tie"><span class="num">C</span><span>can't tell</span></button>
          <button data-c="b"><span class="num">B</span><span>closer to original</span></button>
        </div>
      `;
    } else {
      panel.innerHTML = `
        <div class="rating-panel">
          <button data-r="1"><span class="num">1</span><span>imperceptible</span></button>
          <button data-r="2"><span class="num">2</span><span>I notice</span></button>
          <button data-r="3"><span class="num">3</span><span>I dislike</span></button>
          <button data-r="4"><span class="num">4</span><span>I hate it</span></button>
        </div>
      `;
    }
    function setPanelEnabled(on: boolean) {
      panel.querySelectorAll<HTMLButtonElement>('button').forEach((b) => {
        b.disabled = !on;
      });
    }

    /// Re-evaluate the gate. Called whenever a view changes or a layer lands.
    function refreshGate() {
      const ready = pending <= 0;
      const ok = ready && allSeen();
      setPanelEnabled(ok);
      panel.dataset.gated = ok ? 'no' : 'yes';
      if (gateHint) {
        const missing = requiredViews().filter((v) => !seen.has(v));
        gateHint.hidden = ok;
        gateHint.textContent = !ready ? 'loading…' : missing.length ? gateHintFor(missing) : '';
      }
      // The pill is suppressed while the gate hint is up, so it has to be
      // re-evaluated whenever the gate changes — not only when zoom or mode do.
      updateHint();
    }

    /// Say how to see the arms that are still unseen, in the vocabulary of the
    /// mode the observer is actually in.
    ///
    /// "look at B first" is only meaningful under `tap`, where an A/B/Original
    /// control is on screen to look with. Under `hold` there is no such control
    /// — you press a half of the frame — so the same sentence names a button
    /// that does not exist and leaves the observer stuck at a disabled panel
    /// with no idea what it wants.
    function gateHintFor(missing: View[]): string {
      const names = missing.map((v) => (v === 'a' ? 'A' : 'B'));
      if (inputMode === 'tap') return `look at ${names.join(' and ')} first`;
      // A single-stimulus trial has no left/right split to describe — under
      // `hold` either half shows the one image, and under `buttons` either
      // button does. Naming a side here would send someone to press a
      // particular half for an arm that does not exist. Checked before the
      // per-mode wording, because it applies to both.
      if (!isPair) {
        return inputMode === 'buttons'
          ? 'hold any mouse button to see the compressed image first'
          : 'press and hold to see the compressed image first';
      }
      if (inputMode === 'buttons') {
        // One arm per mouse button, so name the button rather than the arm.
        const how = missing
          .map((v) => (v === 'a' ? 'left' : 'right'))
          .join(' and the ');
        return `hold the ${how} button to see ${names.join(' and ')} first`;
      }
      const how = missing.map((v) => (v === 'a' ? 'left' : 'right')).join(' and ');
      return `press and hold the ${how} half to see ${names.join(' and ')} first`;
    }
    // Answering before the image is on screen would record a judgement of
    // something never seen — and the same is true of the arm you never looked
    // at, which is what `refreshGate` adds.
    setPanelEnabled(false);

    /// How long the observer has spent actively holding a variant up.
    ///
    /// Not total trial time, and not `msOnView.a + .b`: under `tap` A *is* the
    /// resting view, so that sum grows while someone sits doing nothing. What
    /// this measures is time on any view that is not the resting one — i.e.
    /// time with something pressed — which is the same quantity in all three
    /// modes even though the resting view differs.
    function heldMsNow(): number {
      let held = 0;
      for (const v of views) if (v !== resting) held += state.msOnView[v];
      // The interval still open counts, or one long unbroken press would never
      // register — which is precisely the observer this is meant to catch.
      if (viewShownAt !== null && currentSrc !== resting) {
        held += performance.now() - viewShownAt;
      }
      return held;
    }

    /// After a long comparison, say that "can't tell" is a real answer.
    ///
    /// Someone still flicking A against B after this much holding is at their
    /// discrimination threshold, where the truthful answer is a tie. But the
    /// button reads as giving up, so people grind on and eventually guess — and
    /// a guess recorded as a preference is worse data than a recorded tie.
    /// Davidson's model has a tie term precisely so "these look the same to me"
    /// is an outcome rather than noise.
    ///
    /// This is still a nudge toward one specific response, on exactly the
    /// trials where the answer is hardest, so it is recorded per response
    /// (`cant_tell_hint_ms`, migration 0021) and never fired twice.
    let nudgeTimer: number | null = null;
    const stopNudgeTimer = () => {
      if (nudgeTimer !== null) {
        clearInterval(nudgeTimer);
        nudgeTimer = null;
      }
    };
    stopNudge = stopNudgeTimer;
    if (isPair) {
      nudgeTimer = window.setInterval(() => {
        if (submitted || state.cantTellHintMs !== null) return stopNudgeTimer();
        // Not before the trial is up, and not before they have seen both arms
        // — suggesting a tie to someone who has not looked at B yet is telling
        // them to give up on a comparison they have not made.
        if (state.shownAt === 0 || !allSeen()) return;
        if (heldMsNow() < CANT_TELL_HINT_AFTER_HELD_MS) return;
        state.cantTellHintMs = Math.round(performance.now() - state.shownAt);
        panel.querySelector<HTMLButtonElement>('button[data-c="tie"]')?.classList.add('nudge');
        stopNudgeTimer();
      }, 500);
    }

    const commit = (choice: string) => {
      if (submitted || state.shownAt === 0) return;
      // The keyboard bypasses the disabled buttons, so the gate is enforced
      // here too rather than only in the UI.
      if (!allSeen()) return;
      submitted = true;
      stopNudgeTimer();
      detachKeys?.();
      detachKeys = null;
      closeViewAccounting(performance.now());
      setPanelEnabled(false);
      void submit(choice, state, trial, layers[currentSrc], viewport);
    };

    panel.querySelectorAll<HTMLButtonElement>('button').forEach((b) => {
      b.addEventListener('click', () => commit(b.dataset.r ?? b.dataset.c!));
    });

    // ---- keyboard -------------------------------------------------------
    //
    // Letters commit, arrows look, numbers zoom or rate. The split follows the
    // pointer story: what you press to *inspect* is separate from what you
    // press to *answer*, so `a` never means both "show me A" and "A is my
    // answer" in the same trial type.
    //
    // One deliberate asymmetry: on a single-stimulus trial `1`–`4` submit the
    // rating, matching the numerals printed on the buttons, so they cannot also
    // be the zoom ladder. On pair trials nothing owns the digits, so there they
    // do select magnification. `+` / `-` / `0` zoom in every trial type, which
    // is the mapping to reach for if you want one that never changes meaning.

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      const t = e.target as HTMLElement | null;
      if (t && /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName)) return;
      if (document.querySelector('.scrim')) return; // a dialog owns the keyboard
      const k = e.key;

      if (k === '?') {
        e.preventDefault();
        toggleKeyHelp();
        return;
      }
      if (k === 'u' || k === 'U') {
        e.preventDefault();
        void undoLast();
        return;
      }
      if (k === 'Escape') {
        root.querySelector('.key-help')?.remove();
        return;
      }
      if (k === 'i' || k === 'I') {
        e.preventDefault();
        void toggleInfo();
        return;
      }

      state.keyboardUsed = true;

      // Arrows and space are HELD, not tapped: they go through the same stack
      // as the mouse buttons, so ordering behaves identically on either input.
      const btn = buttonForKey(k);
      if (btn) {
        e.preventDefault();
        // `repeat` fires continuously while a key is down; re-pressing would
        // be idempotent anyway, but this keeps reveal accounting honest.
        if (!e.repeat) {
          holds.press(`k${btn}`, viewForPress(btn, { mode: inputMode, isPair, half: null }));
          applyHolds();
        }
        return;
      }

      switch (k) {
        case 'ArrowUp':
        case '+':
        case '=':
          e.preventDefault();
          stepZoom(1);
          return;
        case 'ArrowDown':
        case '-':
        case '_':
          e.preventDefault();
          stepZoom(-1);
          return;
        case '0':
          e.preventDefault();
          applyZoom(1);
          return;
      }

      if (isPair) {
        if (k === 'a' || k === 'A') return commit('a');
        if (k === 'b' || k === 'B') return commit('b');
        if (k === 'c' || k === 'C' || k === 't' || k === 'T') return commit('tie');
        const z = Number(k);
        if (ZOOM_LADDER.includes(z)) {
          e.preventDefault();
          applyZoom(z);
        }
        return;
      }
      if (k >= '1' && k <= '4') {
        e.preventDefault();
        commit(k);
      }
    };

    const onKeyUp = (e: KeyboardEvent) => {
      const btn = buttonForKey(e.key);
      if (!btn) return;
      holds.release(`k${btn}`);
      applyHolds();
    };

    window.addEventListener('keydown', onKeyDown);
    window.addEventListener('keyup', onKeyUp);
    detachKeys = () => {
      window.removeEventListener('keydown', onKeyDown);
      window.removeEventListener('keyup', onKeyUp);
    };

    /// Everything needed to identify what is on screen, as `label: value` lines.
    ///
    /// Exists because an observer who meets a corrupt encode or an artefact
    /// nobody can explain has no way to say *which* image they mean. "The B one
    /// with the green band" is not a bug report; an encoding id is. The trial
    /// id and the build commit are here for the same reason — without them a
    /// report cannot be located in the data or attributed to a version.
    const identifiers = (): Array<[string, string]> => {
      const bytes = (n: number) => `${n} B (${(n / 1024).toFixed(1)} KiB)`;
      const arm = (tag: string, e: NonNullable<TrialPayload['a']>): Array<[string, string]> => [
        [`${tag} encoding`, e.encoding_id],
        [`${tag} codec`, `${e.codec}${e.quality === null ? '' : ` q${e.quality}`}`],
        [`${tag} bytes`, bytes(e.bytes)],
        [`${tag} url`, new URL(e.url, location.origin).href],
      ];
      return [
        ['trial', trial.trial_id],
        ['kind', trial.kind],
        ['session', sessionId],
        ['study', loadStudyId() ?? '(default)'],
        ...(trial.source_filename ? [['source file', trial.source_filename] as [string, string]] : []),
        ['source sha256', trial.source_hash],
        ['source size', `${trial.source_w} x ${trial.source_h}`],
        ['corpus', trial.source_corpus ?? 'unknown'],
        ['license', `${trial.source_license_label} (${trial.source_license_id})`],
        ['original url', new URL(trial.source_url, location.origin).href],
        ...arm('A', trial.a),
        ...(trial.b ? arm('B', trial.b) : []),
        ['input mode', inputMode],
        ['magnification', `${zoomFactor}x`],
        ['device pixel ratio', String(devicePixelRatio)],
        ['build', buildCommit ?? 'unknown'],
      ];
    };

    const toggleInfo = () => {
      const existing = root.querySelector('.key-help.info-help');
      if (existing) {
        existing.remove();
        return;
      }
      const rows = identifiers();
      const panel = document.createElement('div');
      panel.className = 'key-help info-help';
      panel.innerHTML = `
        <div class="key-help-card info-card">
          <h3>What am I looking at?</h3>
          <dl class="info-list">${rows
            .map(
              ([k, v]) =>
                `<dt data-row="${escapeAttr(k)}">${escapeHtml(k)}</dt>` +
                `<dd data-row="${escapeAttr(k)}"><code>${escapeHtml(v)}</code></dd>`,
            )
            .join('')}</dl>
          <div class="row">
            <button id="info-copy" class="primary">Copy all</button>
            <button id="info-close">Close</button>
          </div>
        </div>`;
      root.querySelector('.trial')!.appendChild(panel);
      // Filled in when it arrives rather than awaited before opening: the panel
      // is what someone reaches for mid-report, and it must not wait on the
      // network to appear.
      if (!buildCommit) {
        void loadBuildCommit().then((c) => {
          const cell = panel.querySelector<HTMLElement>('[data-row="build"] code');
          if (c && cell) cell.textContent = c;
        });
      }
      panel.addEventListener('click', (e) => {
        if (e.target === panel) panel.remove();
      });
      panel.querySelector<HTMLButtonElement>('#info-close')!.addEventListener('click', () =>
        panel.remove(),
      );
      const copyBtn = panel.querySelector<HTMLButtonElement>('#info-copy')!;
      copyBtn.addEventListener('click', async () => {
        // Read back from the DOM, not from `rows`: the build commit is patched
        // in after the panel opens, so copying the captured array would hand
        // someone a record saying "unknown" while the screen showed the sha.
        const text = [...panel.querySelectorAll<HTMLElement>('.info-list dt')]
          .map((dt) => {
            const dd = dt.nextElementSibling as HTMLElement | null;
            return `${dt.textContent}: ${dd?.textContent ?? ''}`;
          })
          .join('\n');
        try {
          await navigator.clipboard.writeText(text);
          copyBtn.textContent = 'Copied';
        } catch {
          // Clipboard access is denied on insecure origins and in some
          // embedded browsers. Select the text so it can still be copied by
          // hand rather than leaving a button that silently does nothing.
          const sel = window.getSelection();
          const range = document.createRange();
          range.selectNodeContents(panel.querySelector('.info-list')!);
          sel?.removeAllRanges();
          sel?.addRange(range);
          copyBtn.textContent = 'Select + copy';
        }
      });
    };

    const toggleKeyHelp = () => {
      const existing = root.querySelector('.key-help');
      if (existing) {
        existing.remove();
        return;
      }
      const rows = isPair
        ? [
            ['A / B / C', 'answer: A closer, B closer, can’t tell'],
            ['u', 'take back the previous answer'],
            ['← →', 'cycle A → B → Original'],
            ['space (hold)', 'peek at the original'],
            ['1 – 8', 'magnify by that whole factor'],
          ]
        : [
            ['1 – 4', 'answer: imperceptible → I hate it'],
            ['u', 'take back the previous answer'],
            ['← →', 'switch compressed ↔ original'],
            ['space (hold)', 'peek at the original'],
            ['+ / − / wheel', 'magnify in / out (whole steps)'],
          ];
      const help = document.createElement('div');
      help.className = 'key-help';
      help.innerHTML = `
        <div class="key-help-card">
          <h3>Keyboard</h3>
          <dl>${rows.map(([k, v]) => `<dt>${k}</dt><dd>${v}</dd>`).join('')}</dl>
          <p class="muted">0 resets magnification · ? or Esc closes this</p>
        </div>
      `;
      help.addEventListener('click', () => help.remove());
      root.querySelector('.trial')!.appendChild(help);
    };
    root.querySelector<HTMLButtonElement>('#keys-btn')!.addEventListener('click', toggleKeyHelp);
    showKeyHelp = toggleKeyHelp;
    root.querySelector<HTMLButtonElement>('#info-btn')!.addEventListener('click', () =>
      void toggleInfo(),
    );
    root.querySelector<HTMLButtonElement>('#undo-btn')!.addEventListener('click', () => void undoLast());
  };

  /// Paint the lap bar for the trial currently on screen.
  ///
  /// Hidden until the first comparison is answered: a full-width empty bar on
  /// arrival is a demand, and on a rating-only study it would never move at all
  /// — comparisons are what the threshold counts, so a 4-tier rating correctly
  /// leaves it alone.
  function paintLap() {
    const bar = root.querySelector<HTMLElement>('#lap');
    const fill = root.querySelector<HTMLElement>('#lap-fill');
    if (!bar || !fill || !lapProgress || lapProgress.per <= 0) return;
    const into = lapProgress.done % lapProgress.per;
    const lap = Math.floor(lapProgress.done / lapProgress.per);
    // A completed lap reads as full, not as empty: `done % per === 0` right
    // after crossing would otherwise snap the bar to zero at the exact moment
    // it should look finished.
    const shown = into === 0 && lapProgress.done > 0 ? lapProgress.per : into;
    bar.hidden = lapProgress.done === 0;
    fill.style.width = `${(shown / lapProgress.per) * 100}%`;
    fill.classList.toggle('complete', shown === lapProgress.per);
    const left = lapProgress.per - shown;
    bar.title =
      lap === 0
        ? `${left} more comparisons and your ratings can be reliability-checked`
        : `${lapProgress.done} comparisons · ${left} to the next mark`;
  }

  /// Mark a completed lap.
  ///
  /// The first one is the one that means something — it is where this
  /// observer's data becomes screenable — so it says so. Later laps are
  /// momentum, and are not dressed up as more than that.
  /// Where in a lap a milestone fires.
  ///
  /// Front-loaded on purpose: the first one lands almost immediately so a new
  /// observer learns the threshold exists while they still have the patience to
  /// care, and the rest cluster near the end where "nearly there" is true. A
  /// milestone every few answers would be noise, and noise stops being a
  /// reward.
  const LAP_MILESTONES = [2, 10, 15, 20];

  /// What to say at each mark on the FIRST lap.
  ///
  /// The wording has to stay honest: below 20 comparisons an observer's
  /// reliability cannot be estimated (`crowd_bt::MIN_OBS_FOR_ETA`), so their
  /// answers are stored but cannot be weighted or checked. "Your ratings start
  /// counting at 20" is the plain version of that, and it is true.
  ///
  /// And SHORT. On a 304px cover screen a long sentence wraps to four lines,
  /// which turns a notice pinned to the header band into one reaching down over
  /// the picture. Brevity is the constraint that keeps it off the stimulus.
  function milestoneText(into: number, per: number): string {
    const left = per - into;
    switch (into) {
      case 2:
        return `${left} more and your ratings start counting.`;
      case 10:
        return `Halfway — ${left} more to count.`;
      case 15:
        return `Nearly there: ${left} more.`;
      default:
        return left === 0
          ? `You're in — your ratings now count.`
          : `${left} to go.`;
    }
  }

  /// A milestone notification.
  ///
  /// Placement, timing, fade and tap-to-dismiss all live in `notify.ts` — this
  /// only decides what to say and when.
  function showMilestone(into: number, per: number, firstLap: boolean) {
    const shown = into === 0 ? per : into;
    notify({
      badge: `${shown}/${per}`,
      text: firstLap
        ? milestoneText(shown, per)
        : `${shown} more comparisons this round — thank you.`,
      tone: 'good',
    });
  }

  /// Reopen the previous trial so its answer can be corrected.
  ///
  /// Re-renders that trial with its gate reset — the observer has to look at
  /// both arms again before re-answering, which is the point: an undo is for
  /// "I hit the wrong button", not for changing an answer without re-examining.
  const undoLast = async () => {
    if (!lastAnswered) return;
    const { trial } = lastAnswered;
    lastAnswered = null;
    // The trial we were about to serve is abandoned; a fresh one is drawn after
    // the correction, chosen with the corrected answer in hand.
    trialCount = Math.max(0, trialCount - 1);
    renderTrial(trial);
  };

  const submit = async (
    choice: string,
    state: TrialState,
    trial: TrialPayload,
    img: HTMLImageElement,
    viewport: HTMLElement,
  ) => {
    const dwell = state.shownAt > 0 ? performance.now() - state.shownAt : 0;
    const cond = captureTrial(img, calib.css_px_per_mm, calib.viewing_distance_cm);
    try {
      const ack = await recordResponse(trial.trial_id, {
        choice,
        dwell_ms: Math.round(dwell),
        reveal_count: state.revealCount,
        reveal_ms_total: Math.round(state.revealMsTotal),
        zoom_used: state.zoomUsed,
        pan_count: state.panCount,
        pan_distance_css: Math.round(state.panDistanceCss),
        zoom_factor: state.zoomFactor,
        input_mode: inputMode,
        keyboard_used: state.keyboardUsed,
        ui_ready_ms: state.uiReadyMs,
        switch_count: state.switchCount,
        ms_on_a: Math.round(state.msOnView.a),
        ms_on_b: Math.round(state.msOnView.b),
        ms_on_ref: Math.round(state.msOnView.ref),
        cant_tell_hint_ms: state.cantTellHintMs,
        ...panGeometry(img, viewport),
        ...cond,
      });
      if (ack) {
        const before = lapProgress?.done ?? 0;
        lapProgress = { done: ack.total_comparisons, per: ack.comparisons_per_lap };
        // A lap completes when the count crosses a multiple of the threshold.
        // Compared against the previous value rather than tested for equality,
        // so a comparison that lands while the tab was backgrounded still
        // registers instead of being skipped over.
        const per = ack.comparisons_per_lap;
        if (per > 0 && ack.total_comparisons > before) {
          // Position within the lap, with a completed lap reading as `per`
          // rather than snapping back to 0 at the moment it should feel done.
          const pos = (n: number) => {
            const into = n % per;
            return into === 0 && n > 0 ? per : into;
          };
          const now = pos(ack.total_comparisons);
          const was = before === 0 ? 0 : pos(before);
          // Crossed rather than equalled: an answer that lands while the tab is
          // backgrounded, or a milestone stepped over by a lap boundary, still
          // fires instead of being skipped.
          const hit = LAP_MILESTONES.filter((m) => m > was && m <= now).pop();
          if (hit !== undefined) {
            const firstLap = ack.total_comparisons <= per;
            showMilestone(hit === per ? 0 : hit, per, firstLap);
            if (hit === per) {
              const bar = root.querySelector<HTMLElement>('#lap');
              bar?.classList.add('celebrate');
              window.setTimeout(() => bar?.classList.remove('celebrate'), 2400);
            }
          }
        }
      }
    } catch (e) {
      console.warn('record failed', e);
    }
    lastAnswered = { trial, choice };
    trialCount += 1;
    if (trialCount > 0 && trialCount % 25 === 0) {
      renderBreak(() => fetchAndRender());
    } else {
      void fetchAndRender();
    }
  };

  const renderBreak = (onResume: () => void) => {
    let remaining = 30;
    root.innerHTML = `
      <div class="screen center">
        <h1>Take a 30 s break</h1>
        <p class="muted">Look out a window or just blink.</p>
        <p style="font-size: 3rem; margin: 0;" id="t">${remaining}</p>
        <button id="resume" class="primary" disabled>Resume</button>
      </div>
    `;
    const t = root.querySelector<HTMLParagraphElement>('#t')!;
    const btn = root.querySelector<HTMLButtonElement>('#resume')!;
    const interval = setInterval(() => {
      remaining -= 1;
      t.textContent = `${Math.max(0, remaining)}`;
      if (remaining <= 0) {
        clearInterval(interval);
        btn.disabled = false;
      }
    }, 1000);
    btn.addEventListener('click', () => {
      clearInterval(interval);
      onResume();
    });
  };

  const openMenu = () => {
    const scrim = document.createElement('div');
    scrim.className = 'scrim';
    const modes = availableInputModes();
    scrim.innerHTML = `
      <div class="card menu-card">
        <h2>Pause</h2>
        <p class="muted">You've contributed ${trialCount} ratings this session. Thanks!</p>

        <div class="menu-section">
          <label for="menu-study">Study</label>
          <select id="menu-study"><option>loading…</option></select>
          <p class="muted menu-hint">Switching starts a new session on that study.</p>
        </div>

        <div class="menu-section">
          <label for="menu-mode">How you compare</label>
          <select id="menu-mode">
            ${modes
              .map(
                (m) =>
                  `<option value="${m}"${m === inputMode ? ' selected' : ''}>${INPUT_MODE_LABELS_LONG[m]}</option>`,
              )
              .join('')}
          </select>
        </div>

        <div class="menu-section">
          <button id="menu-calibrate">Re-measure screen size</button>
          <button id="menu-instructions">Re-read the instructions</button>
          <button id="menu-leaderboard">Reviewer leaderboard</button>
          <button id="menu-admin" hidden>Admin</button>
          <button id="menu-account">Sign in</button>
          <button id="menu-keys">Keyboard shortcuts</button>
        </div>

        <!-- Where the leaderboard renders. Inline rather than a second overlay:
             the menu is already a modal, and stacking one on another on a phone
             leaves no obvious way back. -->
        <div id="menu-body"></div>

        <div class="choice-row">
          <button id="continue" class="primary">Keep going</button>
          <button id="end" class="danger">End session</button>
        </div>
      </div>
    `;
    document.body.appendChild(scrim);
    const close = () => scrim.remove();
    // Clicking the backdrop dismisses; clicking the card must not.
    scrim.addEventListener('click', (e) => {
      if (e.target === scrim) close();
    });

    const studySel = scrim.querySelector<HTMLSelectElement>('#menu-study')!;
    void listStudies()
      .then((studies) => {
        const current = loadStudyId();
        studySel.innerHTML = studies
          .map(
            (st) =>
              `<option value="${st.id}"${st.id === current ? ' selected' : ''}>${escapeHtml(st.label)}</option>`,
          )
          .join('');
        studySel.addEventListener('change', () => {
          // A session belongs to one study — its trials and responses are all
          // filed under it — so switching cannot mutate the current one. Record
          // the choice, end this session cleanly, and let the caller start a
          // fresh one.
          saveStudyId(studySel.value);
          close();
          aborted = true;
          detachKeys?.();
          detachKeys = null;
          onSwitchStudy();
        });
      })
      .catch(() => {
        studySel.innerHTML = '<option>unavailable</option>';
        studySel.disabled = true;
      });

    const modeSel = scrim.querySelector<HTMLSelectElement>('#menu-mode')!;
    modeSel.addEventListener('change', () => {
      const v = modeSel.value;
      if (!isInputMode(v)) return;
      inputMode = v;
      saveInputMode(v);
      close();
      // Re-render so the resting view and bindings match the new mode. Cheap:
      // every variant is already decoded.
      if (currentTrial) renderTrial(currentTrial);
    });

    scrim.querySelector<HTMLButtonElement>('#menu-calibrate')!.addEventListener('click', () => {
      close();
      aborted = true;
      detachKeys?.();
      detachKeys = null;
      onRecalibrate();
    });
    // Signing in from the menu, and out again. Kept here rather than only on the
    // front page because that is where somebody notices they are anonymous —
    // mid-session, looking at the board, wondering why they are not on it.
    void (async () => {
      const btn = scrim.querySelector<HTMLButtonElement>('#menu-account');
      const admin = scrim.querySelector<HTMLButtonElement>('#menu-admin');
      if (!btn) return;
      const me = await whoami().catch(() => null);
      if (me?.signed_in) {
        btn.textContent = `Sign out (${me.email ?? 'signed in'})`;
        btn.addEventListener('click', async () => {
          await signOut();
          close();
          if (currentTrial) renderTrial(currentTrial);
        });
        if (me.is_admin && admin) {
          admin.hidden = false;
          admin.addEventListener('click', () => {
            close();
            void showAdmin(root, () => {
              if (currentTrial) renderTrial(currentTrial);
            });
          });
        }
      } else {
        btn.textContent = 'Sign in to keep your reviewer name';
        btn.addEventListener('click', () => {
          close();
          openSignInModal();
        });
      }
    })();

    scrim
      .querySelector<HTMLButtonElement>('#menu-instructions')!
      .addEventListener('click', () => {
        close();
        // `force`, because this IS the deliberate re-read: the once-per-session
        // rule exists to stop it appearing unbidden, not to make it unreachable.
        void showInstructions(root, { returning: true }).then(() => {
          if (currentTrial) renderTrial(currentTrial);
        });
      });
    scrim
      .querySelector<HTMLButtonElement>('#menu-leaderboard')!
      .addEventListener('click', () => void showLeaderboard(scrim));
    scrim.querySelector<HTMLButtonElement>('#menu-keys')!.addEventListener('click', () => {
      close();
      showKeyHelp?.();
    });

    scrim.querySelector<HTMLButtonElement>('#continue')!.addEventListener('click', close);
    scrim.querySelector<HTMLButtonElement>('#end')!.addEventListener('click', () => {
      close();
      aborted = true;
      detachKeys?.();
      detachKeys = null;
      renderDone();
    });
  };

  /// End of session: what you did, and where it sits.
  ///
  /// "You contributed N ratings. Close this tab" gave a volunteer no reason to
  /// come back and no way to tell whether the N was any good. The board answers
  /// both — and the reliability column beside the count is the honest framing:
  /// volume alone is not the contribution.
  const renderDone = () => {
    root.innerHTML = `
      <div class="screen center done">
        <h1>Thank you</h1>
        <p>You contributed <strong>${trialCount}</strong> ${
          trialCount === 1 ? 'rating' : 'ratings'
        } this session.</p>
        <div id="done-board" class="muted">Loading your stats…</div>
        <div class="row">
          <button id="done-again" class="primary">Rate some more</button>
          <a id="done-home" href="/">Back to the front page</a>
        </div>
      </div>
    `;
    root.querySelector<HTMLButtonElement>('#done-again')!.addEventListener('click', () => {
      location.href = '/rate';
    });
    void renderDoneBoard();
  };

  const renderDoneBoard = async () => {
    const host = root.querySelector<HTMLElement>('#done-board');
    if (!host) return;
    try {
      const board = await listLeaderboard(getObserverId());
      const me = board.find((r) => r.is_you);
      const pct = (v: number | null) => (v === null ? '—' : `${Math.round(v * 100)}%`);
      const rows = [...board.filter((r) => r.is_you), ...board.filter((r) => !r.is_you)].slice(0, 8);
      host.innerHTML = `
        ${
          me
            ? `<p class="done-mine">You are <code>${escapeHtml(me.handle)}</code> —
                 <b>${me.trials.toLocaleString()}</b> ratings all told,
                 ${pct(me.self_agreement)} agreement with yourself,
                 ${(me.active_seconds / 3600).toFixed(1)}h engaged.</p>`
            : '<p class="muted">Your first ratings are in — the board updates as they land.</p>'
        }
        ${
          (await whoami().catch(() => null))?.signed_in
            ? ''
            : `<p class="signin-nudge">You are rating as a guest. <button id="done-signin"
                 class="linkish">Sign in with email</button> to keep this reviewer name
                 across devices — and so your sessions stay linked to you if we need to ask
                 about one later.</p>`
        }
        <table class="board">
          <thead><tr><th>Reviewer</th><th>Ratings</th><th>Self-agree</th><th>Hours</th></tr></thead>
          <tbody>${rows
            .map(
              (r) => `<tr class="${r.is_you ? 'me' : ''}">
                <td><code>${escapeHtml(r.handle)}</code>${r.is_you ? ' <span class="you">you</span>' : ''}</td>
                <td>${r.trials.toLocaleString()}</td>
                <td>${pct(r.self_agreement)}</td>
                <td>${(r.active_seconds / 3600).toFixed(1)}</td>
              </tr>`,
            )
            .join('')}</tbody>
        </table>`;
      host
        .querySelector<HTMLButtonElement>('#done-signin')
        ?.addEventListener('click', () => openSignInModal());
    } catch {
      host.innerHTML = '<p class="muted">Stats are unavailable right now.</p>';
    }
  };

  return {
    async start() {
      await fetchAndRender();
    },
    end() {
      aborted = true;
      detachKeys?.();
      detachKeys = null;
      stopNudge?.();
      stopNudge = null;
      renderDone();
    },
  };
}

/// Which build is serving this page, for a bug report to be attributable.
///
/// Fetched lazily and cached: only needed when someone opens the identifier
/// panel, so a session that never does pays nothing.
///
/// From `/api/stats`, NOT an export manifest. A manifest computes its export to
/// report a row count, so it gets slower as the study fills up — blocking the
/// panel on it made opening take seconds once there was real data, which is
/// exactly when someone is trying to report a bad encode. `/api/stats` is
/// constant-cost.
let buildCommit: string | null = null;
async function loadBuildCommit(): Promise<string | null> {
  if (buildCommit) return buildCommit;
  try {
    const r = await fetch('/api/stats');
    if (!r.ok) return null;
    const j = (await r.json()) as { build_commit?: string };
    buildCommit = j.build_commit ?? null;
  } catch {
    /* offline or blocked: the panel says "unknown" rather than failing to open */
  }
  return buildCommit;
}

/// The reviewer board, shown from the pause menu.
///
/// Deliberately not a ranking by volume: sorting on trial count alone rewards
/// clicking through, which is the behaviour the attention checks exist to
/// catch. Self-agreement sits next to the count for exactly that reason — high
/// volume with low self-agreement is noise, not data. Handles are derived and
/// unreversible (see `src/handle.rs`), so a public board leaks no addresses.
async function showLeaderboard(host: HTMLElement): Promise<void> {
  const body = host.querySelector<HTMLElement>('#menu-body');
  if (!body) return;
  body.innerHTML = `<p class="muted">Loading the board…</p>`;
  let rows: LeaderboardRow[];
  try {
    rows = await listLeaderboard();
  } catch (e) {
    body.innerHTML = `<p class="muted">Couldn't load the leaderboard: ${escapeHtml(
      (e as Error).message,
    )}</p>`;
    return;
  }
  if (!rows.length) {
    body.innerHTML = `<p class="muted">No reviewers on the board yet.</p>`;
    return;
  }
  const pct = (v: number | null) => (v === null ? '—' : `${Math.round(v * 100)}%`);
  const num = (v: number | null, d = 1) => (v === null ? '—' : v.toFixed(d));
  body.innerHTML = `
    <div class="board-wrap">
      <table class="board">
        <thead><tr>
          <th>Reviewer</th><th>Trials</th><th>Days</th>
          <th title="Agreement with themselves on re-served pairs — the ceiling any metric could reach against this reviewer">Self-agree</th>
          <th title="Attention-check pass rate">Checks</th>
          <th title="Median seconds per judgement">s/trial</th>
          <th title="Median view swaps per trial — how much comparing they actually did. Over instrumented trials only; older responses predate the counter.">Swaps</th>
          <th title="Engaged time: gaps between consecutive answers within a session, each capped so a break is not billed, plus the first answer's dwell">Hours</th>
        </tr></thead>
        <tbody>${rows
          .map(
            (r) => `<tr>
              <td><code>${escapeHtml(r.handle)}</code></td>
              <td>${r.trials}</td>
              <td>${r.active_days}</td>
              <td>${pct(r.self_agreement)}${r.repeat_pairs ? '' : ' <span class="muted">(n/a)</span>'}</td>
              <td>${pct(r.golden_pass_rate)}</td>
              <td>${num(r.median_seconds)}</td>
              <td>${num(r.median_switches, 0)}${
                r.instrumented_trials && r.instrumented_trials < r.trials
                  ? ` <span class="muted">(${r.instrumented_trials})</span>`
                  : ''
              }</td>
              <td>${(r.active_seconds / 3600).toFixed(2)}</td>
            </tr>`,
          )
          .join('')}</tbody>
      </table>
      <p class="muted board-note">Names are derived from a salted hash and cannot be
        reversed. Self-agreement is shown beside volume on purpose: answering a lot
        quickly is only good if the answers are consistent.</p>
      <p class="muted board-note"><b>Hours</b> is engaged time, not wall clock: the
        gap between consecutive answers in a session, with any gap long enough to
        be a break counted only up to the cap, plus the first answer's own dwell.
        It can be recomputed from the response export. A bracketed number beside
        <b>Swaps</b> is how many of that reviewer's trials carry the counter —
        responses recorded before it existed are excluded rather than read as
        zero.</p>
    </div>`;
}

function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);
}

function escapeAttr(s: string): string {
  return escapeHtml(s);
}

/**
 * How much of the stimulus the observer could actually see, and how much of it
 * was off-screen. With 1:1 display mandatory a large stimulus does not fit, so
 * "displayed size" alone no longer says what was looked at.
 */
function panGeometry(img: HTMLImageElement, viewport: HTMLElement) {
  const i = img.getBoundingClientRect();
  const v = viewport.getBoundingClientRect();
  const visibleW = Math.max(0, Math.min(i.right, v.right) - Math.max(i.left, v.left));
  const visibleH = Math.max(0, Math.min(i.bottom, v.bottom) - Math.max(i.top, v.top));
  return {
    pannable_w_css: Math.round(Math.max(0, i.width - v.width)),
    pannable_h_css: Math.round(Math.max(0, i.height - v.height)),
    visible_w_css: Math.round(visibleW),
    visible_h_css: Math.round(visibleH),
  };
}
