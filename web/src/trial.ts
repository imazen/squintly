// Trial loop. Single-stimulus 4-tier ACR by default; pair trials use 3-button
// "A closer / tie / B closer". The reference is always reachable — by button,
// by press-and-hold, by keyboard, or (in `hold` mode) as the resting state.

import { nextTrial, recordResponse, type TrialPayload } from './api';
import { captureTrial, loadCalibration } from './conditions';
import {
  INPUT_MODE_LABELS,
  INPUT_MODES,
  type InputMode,
  inputModeHint,
  loadInputMode,
  saveInputMode,
  supportsHoldMode,
} from './input-mode';

type View = 'a' | 'b' | 'ref';

const ZOOM_LADDER = [1, 2, 4, 8];

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
  /// Time from render to the judged image being painted. Kept separable from
  /// `dwell_ms`: waiting for a decode is not deliberation.
  uiReadyMs: number | null;
}

export interface TrialController {
  start(): Promise<void>;
  end(): void;
}

export function startTrials(root: HTMLElement, sessionId: string): TrialController {
  let aborted = false;
  let trialCount = 0;
  /// Magnification persists across trials in a session — see the note where it
  /// is applied. Recorded per response, so persistence costs no fidelity.
  let zoomFactor = 1;
  let inputMode: InputMode = loadInputMode();
  /// Torn down before each render; a stale listener would drive the previous
  /// trial's closure and submit against a trial that is no longer on screen.
  let detachKeys: (() => void) | null = null;

  const calib = loadCalibration();

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
    detachKeys?.();
    detachKeys = null;

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
      uiReadyMs: null,
    };

    const isPair = trial.kind === 'pair';
    const corpus = trial.source_corpus ?? 'unknown';
    const licId = trial.source_license_id;
    const licLabel = trial.source_license_label;
    const views: View[] = isPair ? ['a', 'b', 'ref'] : ['a', 'ref'];
    const srcFor = (v: View) =>
      v === 'ref' ? trial.source_url : v === 'a' ? trial.a.url : trial.b!.url;

    const modePicker = supportsHoldMode()
      ? `<label class="mode-picker">
           <span class="sr-only">Interaction mode</span>
           <select id="input-mode" aria-label="Interaction mode">
             ${INPUT_MODES.map(
               (m) =>
                 `<option value="${m}"${m === inputMode ? ' selected' : ''}>${INPUT_MODE_LABELS[m]}</option>`,
             ).join('')}
           </select>
         </label>`
      : '';

    root.innerHTML = `
      <div class="trial" data-trial-id="${trial.trial_id}" data-input-mode="${inputMode}">
        <div class="progress">
          <span>Trial ${trialCount + 1}</span>
          <span class="trial-license" data-corpus="${escapeAttr(corpus)}" data-license-id="${escapeAttr(licId)}" title="${escapeAttr(licLabel)}">${escapeHtml(corpus)} · ${escapeHtml(licLabel)}</span>
          <button class="menu-btn" id="menu">menu</button>
        </div>
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
        <div class="trial-controls">
          <div class="view-switch" id="view-switch" role="group" aria-label="Which image">
            ${
              isPair
                ? `<button data-view="a" class="on">A</button>
                   <button data-view="b">B</button>
                   <button data-view="ref">Original</button>`
                : `<button data-view="a" class="on">Compressed</button>
                   <button data-view="ref">Original</button>`
            }
          </div>
          <div class="zoom-switch" id="zoom-switch" role="group" aria-label="Magnification">
            ${ZOOM_LADDER.map(
              (z) => `<button data-zoom="${z}" class="${z === 1 ? 'on' : ''}">${z}×</button>`,
            ).join('')}
          </div>
          ${modePicker}
          <button class="keys-btn" id="keys-btn" aria-label="Keyboard shortcuts" title="Keyboard shortcuts (?)">⌨</button>
        </div>
        <div class="reveal-hint" id="hint"></div>
        <div id="panel"></div>
      </div>
    `;
    const viewport = root.querySelector<HTMLDivElement>('#viewport')!;
    const panel = root.querySelector<HTMLDivElement>('#panel')!;
    const hint = root.querySelector<HTMLDivElement>('#hint')!;
    const status = root.querySelector<HTMLDivElement>('#vp-status')!;
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
    const restingView: View = inputMode === 'hold' ? 'ref' : 'a';
    let currentSrc: View = restingView;
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

    const clampPan = () => {
      pan.x = Math.max(-panLimit.x, Math.min(panLimit.x, pan.x));
      pan.y = Math.max(-panLimit.y, Math.min(panLimit.y, pan.y));
    };
    /// Pan applies to every layer, so a switch lands on the same region — you
    /// are comparing the same part of the picture, not two different parts.
    const applyPan = () => {
      const t = `translate(${pan.x}px, ${pan.y}px)`;
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
      // prevent. Keep the existing limits; `onLayerReady` recomputes for real.
      if (!(w > 0) || !(h > 0)) return;
      // Centred by `margin:auto`, so it can travel half the overflow each way.
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

    const markActive = (host: HTMLElement, attr: string, value: string) => {
      host.querySelectorAll<HTMLButtonElement>('button').forEach((b) => {
        b.classList.toggle('on', b.dataset[attr] === value);
      });
    };

    function updateHint() {
      const bits: string[] = [];
      if (isPannable()) bits.push('drag to explore');
      bits.push(inputModeHint(inputMode, isPair));
      hint.textContent = bits.join(' · ');
      hint.hidden = bits.length === 0;
    }

    // ---- reveal accounting ----------------------------------------------
    //
    // "Reveal" is time the *reference* was on screen, in both modes. Under
    // `tap` that is a deliberate peek; under `hold` it is the resting state
    // and will dominate the trial. Both are recorded the same way and the mode
    // is stored alongside, so an analyst can tell them apart — see migration
    // 0017. Inferring the mode from the magnitude of this number afterwards
    // would be guessing.
    let refShownAt: number | null = null;
    const closeRefAccounting = (now: number) => {
      if (refShownAt !== null) {
        state.revealMsTotal += now - refShownAt;
        refShownAt = null;
      }
    };

    /// Show a given variant. Zero-cost after load: everything is already
    /// decoded, so this only flips which layer is visible.
    const showView = (which: View) => {
      if (which === 'b' && !isPair) return;
      const now = performance.now();
      if (which !== currentSrc) {
        closeRefAccounting(now);
        if (which === 'ref') {
          refShownAt = now;
          state.revealCount += 1;
        }
      }
      currentSrc = which;
      if (which !== 'ref') choiceSrc = which;
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
    };

    // ---- load / decode tracking -----------------------------------------
    let pending = views.length;
    const onLayerReady = (v: View) => {
      sizeLayer(layers[v]);
      pending -= 1;
      if (v === currentSrc) {
        // The judged image is up: start the clock and let the observer answer.
        if (state.shownAt === 0) {
          state.shownAt = performance.now();
          state.uiReadyMs = Math.round(state.shownAt - renderedAt);
        }
        viewport.classList.remove('is-loading');
        status.hidden = true;
        setPanelEnabled(true);
        recomputePanLimits();
      }
      if (pending <= 0) viewport.classList.add('all-ready');
    };

    for (const v of views) {
      const el = layers[v];
      el.addEventListener('load', () => onLayerReady(v));
      el.addEventListener('error', () => {
        pending -= 1;
        if (v === currentSrc) {
          status.innerHTML = `<p class="muted">That image failed to load.</p>`;
          status.hidden = false;
        }
      });
      el.src = srcFor(v);
    }
    showView(restingView);
    // Anything already in cache resolves before the listener attached above.
    for (const v of views) if (layers[v].complete && layers[v].naturalWidth > 0) onLayerReady(v);

    viewSwitch.querySelectorAll<HTMLButtonElement>('button').forEach((b) => {
      b.addEventListener('click', () => {
        const v = b.dataset.view as View | undefined;
        if (v) showView(v);
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
      markActive(zoomSwitch, 'zoom', String(next));
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
      b.addEventListener('click', () => applyZoom(Number(b.dataset.zoom)));
    });
    markActive(zoomSwitch, 'zoom', String(zoom));

    root.querySelector<HTMLSelectElement>('#input-mode')?.addEventListener('change', (e) => {
      const v = (e.target as HTMLSelectElement).value;
      if (v !== 'tap' && v !== 'hold') return;
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
    let pointerId: number | null = null;
    let startX = 0;
    let startY = 0;
    let lastX = 0;
    let lastY = 0;
    let dragging = false;

    /// Which half of the frame a press landed in.
    const pressedHalf = (e: PointerEvent): 'left' | 'right' => {
      const r = viewport.getBoundingClientRect();
      return e.clientX < r.left + r.width / 2 ? 'left' : 'right';
    };

    // A long press over an image raises the callout/context menu on both mobile
    // and desktop, which would interrupt the hold exactly when it is the
    // primary gesture.
    viewport.addEventListener('contextmenu', (e) => {
      if (inputMode === 'hold') e.preventDefault();
    });

    viewport.addEventListener('pointerdown', (e: PointerEvent) => {
      if (pointerId !== null) return;
      pointerId = e.pointerId;
      startX = lastX = e.clientX ?? 0;
      startY = lastY = e.clientY ?? 0;
      dragging = false;
      if (inputMode === 'hold') {
        // Which half you press picks the variant — A on the left, B on the
        // right, matching the view switch and the answer buttons. Split on the
        // *viewport*, not the image: this is about where your finger is on the
        // screen, and the image may be panned or larger than the frame.
        //
        // Decided once, on press, and held for the whole gesture. Re-evaluating
        // as the pointer moves would fight panning — a drag that crossed the
        // midline would swap the variant out from under a comparison.
        showView(pressedHalf(e) === 'right' && isPair ? 'b' : 'a');
      } else if (currentSrc !== 'ref') {
        showView('ref');
      }
      try {
        viewport.setPointerCapture(e.pointerId);
      } catch {
        /* no capture (synthetic or already-released pointer); drag still works */
      }
    });

    viewport.addEventListener('pointermove', (e: PointerEvent) => {
      if (e.pointerId !== pointerId) return;
      if (!dragging) {
        const moved = Math.hypot(e.clientX - startX, e.clientY - startY);
        if (moved < DRAG_THRESHOLD_CSS) return;
        dragging = true;
        state.panCount += 1;
        if (isPannable()) viewport.classList.add('panning');
      }
      if (!isPannable()) return;
      const dx = e.clientX - lastX;
      const dy = e.clientY - lastY;
      lastX = e.clientX;
      lastY = e.clientY;
      pan.x += dx;
      pan.y += dy;
      state.panDistanceCss += Math.hypot(dx, dy);
      clampPan();
      applyPan();
    });

    const endPointer = (e: PointerEvent) => {
      if (e.pointerId !== pointerId) return;
      pointerId = null;
      try {
        viewport.releasePointerCapture(e.pointerId);
      } catch {
        /* already released */
      }
      showView(inputMode === 'hold' ? 'ref' : choiceSrc);
      dragging = false;
      viewport.classList.remove('panning');
    };
    viewport.addEventListener('pointerup', endPointer);
    viewport.addEventListener('pointercancel', endPointer);

    // Wheel/pinch zoom detection (we don't actually zoom — we just record).
    viewport.addEventListener('wheel', () => { state.zoomUsed = true; }, { passive: true });
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
    // Answering before the image is on screen would record a judgement of
    // something never seen.
    setPanelEnabled(false);

    const commit = (choice: string) => {
      if (submitted || state.shownAt === 0) return;
      submitted = true;
      detachKeys?.();
      detachKeys = null;
      closeRefAccounting(performance.now());
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
    const cycle = (dir: 1 | -1) => {
      const i = views.indexOf(currentSrc);
      showView(views[(i + dir + views.length) % views.length]);
    };

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
      if (k === 'Escape') {
        root.querySelector('.key-help')?.remove();
        return;
      }

      state.keyboardUsed = true;

      switch (k) {
        case 'ArrowRight':
          e.preventDefault();
          cycle(1);
          return;
        case 'ArrowLeft':
          e.preventDefault();
          cycle(-1);
          return;
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
        case ' ':
          // Hold to peek at the reference; `repeat` fires while held, and
          // re-entering would inflate the reveal count.
          e.preventDefault();
          if (!e.repeat) showView('ref');
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
      if (e.key === ' ') showView(inputMode === 'hold' ? 'ref' : choiceSrc);
    };

    window.addEventListener('keydown', onKeyDown);
    window.addEventListener('keyup', onKeyUp);
    detachKeys = () => {
      window.removeEventListener('keydown', onKeyDown);
      window.removeEventListener('keyup', onKeyUp);
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
            ['← →', 'cycle A → B → Original'],
            ['space (hold)', 'peek at the original'],
            ['1 2 4 8', 'magnify 1× 2× 4× 8×'],
          ]
        : [
            ['1 – 4', 'answer: imperceptible → I hate it'],
            ['← →', 'switch compressed ↔ original'],
            ['space (hold)', 'peek at the original'],
            ['+ / −', 'magnify in / out'],
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
      await recordResponse(trial.trial_id, {
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
        ...panGeometry(img, viewport),
        ...cond,
      });
    } catch (e) {
      console.warn('record failed', e);
    }
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
    scrim.innerHTML = `
      <div class="card">
        <h2>Pause</h2>
        <p class="muted">You've contributed ${trialCount} ratings so far. Thanks!</p>
        <div class="choice-row">
          <button id="continue" class="primary">Keep going</button>
          <button id="end" class="danger">End session</button>
        </div>
      </div>
    `;
    document.body.appendChild(scrim);
    scrim.querySelector<HTMLButtonElement>('#continue')!.addEventListener('click', () => scrim.remove());
    scrim.querySelector<HTMLButtonElement>('#end')!.addEventListener('click', () => {
      scrim.remove();
      aborted = true;
      detachKeys?.();
      detachKeys = null;
      renderDone();
    });
  };

  const renderDone = () => {
    root.innerHTML = `
      <div class="screen center">
        <h1>Thank you</h1>
        <p>You contributed <strong>${trialCount}</strong> ratings.</p>
        <p class="muted">Close this tab when you're ready.</p>
      </div>
    `;
  };

  return {
    async start() {
      await fetchAndRender();
    },
    end() {
      aborted = true;
      detachKeys?.();
      detachKeys = null;
      renderDone();
    },
  };
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
