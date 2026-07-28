// Trial loop. Single-stimulus 4-tier ACR by default; pair trials use 3-button
// "A closer / tie / B closer" with carousel toggle. Hold-the-image to reveal
// the reference (CID22-PTC style).

import { nextTrial, recordResponse, type TrialPayload } from './api';
import { captureTrial, loadCalibration } from './conditions';

interface TrialState {
  shownAt: number;
  revealCount: number;
  revealMsTotal: number;
  revealStartedAt: number | null;
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

  const calib = loadCalibration();

  const fetchAndRender = async () => {
    if (aborted) return;
    let trial: TrialPayload;
    try {
      trial = await nextTrial(sessionId);
    } catch (e) {
      root.innerHTML = `<div class="screen center"><h1>No trials available</h1><p class="muted">${
        (e as Error).message
      }</p></div>`;
      return;
    }
    renderTrial(trial);
  };

  const renderTrial = (trial: TrialPayload) => {
    const state: TrialState = {
      shownAt: 0,
      revealCount: 0,
      revealMsTotal: 0,
      revealStartedAt: null,
      zoomUsed: false,
      panCount: 0,
      panDistanceCss: 0,
      zoomFactor,
    };

    const isPair = trial.kind === 'pair';
    const corpus = trial.source_corpus ?? 'unknown';
    const licId = trial.source_license_id;
    const licLabel = trial.source_license_label;
    root.innerHTML = `
      <div class="trial" data-trial-id="${trial.trial_id}">
        <div class="progress">
          <span>Trial ${trialCount + 1}</span>
          <span class="trial-license" data-corpus="${escapeAttr(corpus)}" data-license-id="${escapeAttr(licId)}" title="${escapeAttr(licLabel)}">${escapeHtml(corpus)} · ${escapeHtml(licLabel)}</span>
          <button class="menu-btn" id="menu">menu</button>
        </div>
        <div class="viewport" id="viewport">
          <img id="stimulus" alt="" decoding="async" />
        </div>
        <div class="trial-controls">
          <div class="view-switch" id="view-switch" role="group" aria-label="Which image">
            ${isPair
              ? `<button data-view="a" class="on">A</button>
                 <button data-view="b">B</button>
                 <button data-view="ref">Original</button>`
              : `<button data-view="a" class="on">Compressed</button>
                 <button data-view="ref">Original</button>`}
          </div>
          <div class="zoom-switch" id="zoom-switch" role="group" aria-label="Magnification">
            ${[1, 2, 4].map((z) => `<button data-zoom="${z}" class="${z === 1 ? 'on' : ''}">${z}×</button>`).join('')}
          </div>
        </div>
        <div class="reveal-hint" id="hint"></div>
        <div id="panel"></div>
      </div>
    `;
    const viewport = root.querySelector<HTMLDivElement>('#viewport')!;
    const img = root.querySelector<HTMLImageElement>('#stimulus')!;
    const panel = root.querySelector<HTMLDivElement>('#panel')!;
    const hint = root.querySelector<HTMLDivElement>('#hint')!;
    const menu = root.querySelector<HTMLButtonElement>('#menu')!;
    menu.addEventListener('click', () => openMenu());

    // ---- 1:1 device pixels, mandatory ------------------------------------
    //
    // The stimulus is rendered at exactly one image pixel per device pixel
    // (CSS size = intrinsic / dpr), and NEVER smaller. Anything larger than
    // the viewport is explored by dragging, not shrunk to fit.
    //
    // This used to `Math.min(1, …)` down to whatever fitted. That silently
    // resampled the stimulus in the browser, so the observer was rating the
    // *browser's* downscale of the encode rather than the encode — the
    // artefacts under test get averaged away, and the effect is strongest
    // exactly where the study cares most (high-DPR phones, large sources).
    // Whatever number came back was not a measurement of the codec.
    //
    // Zooming in beyond 1:1 is acceptable; going below it is not.
    const dpr = window.devicePixelRatio ?? 1;
    const pan = { x: 0, y: 0 }; // CSS px offset from the centred crop
    const panLimit = { x: 0, y: 0 };
    let currentSrc: 'a' | 'b' | 'ref' = 'a';
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

    const clampPan = () => {
      pan.x = Math.max(-panLimit.x, Math.min(panLimit.x, pan.x));
      pan.y = Math.max(-panLimit.y, Math.min(panLimit.y, pan.y));
    };
    const applyPan = () => {
      img.style.transform = `translate(${pan.x}px, ${pan.y}px)`;
    };

    const setSrc = (which: 'a' | 'b' | 'ref') => {
      currentSrc = which;
      img.src = which === 'ref' ? trial.source_url : which === 'a' ? trial.a.url : trial.b!.url;
    };

    img.addEventListener('load', () => {
      const cssW = (img.naturalWidth * zoom) / dpr;
      const cssH = (img.naturalHeight * zoom) / dpr;
      img.style.width = `${cssW}px`;
      img.style.height = `${cssH}px`;
      img.style.maxWidth = 'none'; // the stylesheet's max-width:100% would re-shrink it
      img.style.maxHeight = 'none';
      const rect = viewport.getBoundingClientRect();
      // The image is centred by the flex viewport, so it can travel half the
      // overflow in each direction from centre.
      panLimit.x = Math.max(0, (cssW - rect.width) / 2);
      panLimit.y = Math.max(0, (cssH - rect.height) / 2);
      // Deliberately does NOT reset `pan`: swapping encoded↔reference (or
      // A↔B) must hold the observer on the same region, or they are comparing
      // two different parts of the picture.
      clampPan();
      applyPan();
      viewport.classList.toggle('pannable', isPannable());
      updateHint();
      if (state.shownAt === 0) state.shownAt = performance.now();
    });
    setSrc('a');

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
      if (!isPair) bits.push('or hold the image to compare');
      hint.textContent = bits.join(' · ');
    }

    /// Show a given image. The pan offset is deliberately untouched: switching
    /// between A, B and the original must hold the observer on the same region,
    /// or they are comparing different parts of the picture.
    const showView = (which: 'a' | 'b' | 'ref') => {
      if (which !== 'ref') choiceSrc = which;
      setSrc(which);
      markActive(viewSwitch, 'view', which);
      updateHint();
    };

    viewSwitch.querySelectorAll<HTMLButtonElement>('button').forEach((b) => {
      b.addEventListener('click', () => {
        const v = b.dataset.view as 'a' | 'b' | 'ref' | undefined;
        if (!v) return;
        if (v === 'ref') state.revealCount += 1;
        showView(v);
      });
    });

    const applyZoom = (next: number) => {
      if (next === zoom) return;
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
      // Mid-swap the image reports naturalWidth 0; sizing from that collapses
      // the element and zeroes the pan limits. The load handler re-runs this
      // with real dimensions, so skipping here is safe and avoids the flash.
      if (img.naturalWidth === 0) return;
      // Re-run the load sizing against the new factor.
      const cssW = (img.naturalWidth * zoom) / dpr;
      const cssH = (img.naturalHeight * zoom) / dpr;
      img.style.width = `${cssW}px`;
      img.style.height = `${cssH}px`;
      const rect = viewport.getBoundingClientRect();
      panLimit.x = Math.max(0, (cssW - rect.width) / 2);
      panLimit.y = Math.max(0, (cssH - rect.height) / 2);
      clampPan();
      applyPan();
      viewport.classList.toggle('pannable', isPannable());
      updateHint();
    };

    zoomSwitch.querySelectorAll<HTMLButtonElement>('button').forEach((b) => {
      b.addEventListener('click', () => {
        const z = Number(b.dataset.zoom);
        if (Number.isFinite(z) && z >= 1) applyZoom(z);
      });
    });
    markActive(zoomSwitch, 'zoom', String(zoom));

    // Press-and-hold on the image is a shortcut for "show me the original",
    // available in BOTH trial types. Pair trials previously had no way to see
    // the reference at all, while asking which encode was "closer to original".
    const startReveal = () => {
      if (currentSrc === 'ref') return;
      state.revealStartedAt = performance.now();
      state.revealCount += 1;
      root.querySelector('.trial')?.classList.add('revealing');
      showView('ref');
    };
    const endReveal = () => {
      if (state.revealStartedAt !== null) {
        state.revealMsTotal += performance.now() - state.revealStartedAt;
        state.revealStartedAt = null;
      }
      root.querySelector('.trial')?.classList.remove('revealing');
      showView(choiceSrc);
    };

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

    viewport.addEventListener('pointerdown', (e: PointerEvent) => {
      if (pointerId !== null) return;
      pointerId = e.pointerId;
      startX = lastX = e.clientX ?? 0;
      startY = lastY = e.clientY ?? 0;
      dragging = false;
      startReveal();
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
      endReveal();
      dragging = false;
      viewport.classList.remove('panning');
    };
    viewport.addEventListener('pointerup', endPointer);
    viewport.addEventListener('pointercancel', endPointer);

    // Wheel/pinch zoom detection (we don't actually zoom — we just record).
    viewport.addEventListener('wheel', () => { state.zoomUsed = true; }, { passive: true });
    viewport.addEventListener('gesturestart', () => { state.zoomUsed = true; });

    // Build response panel
    if (isPair) {
      panel.innerHTML = `
        <div class="pair-panel">
          <button data-c="a"><span class="num">A</span><span>closer to original</span></button>
          <button data-c="tie"><span class="num">≈</span><span>can't tell</span></button>
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
    panel.querySelectorAll<HTMLButtonElement>('button').forEach((b) => {
      b.addEventListener('click', () => {
        const choice = b.dataset.r ?? b.dataset.c!;
        submit(choice, state, trial, img, viewport);
      });
    });
  };

  const submit = async (
    choice: string,
    state: TrialState,
    trial: TrialPayload,
    img: HTMLImageElement,
    viewport: HTMLElement,
  ) => {
    if (state.revealStartedAt !== null) {
      state.revealMsTotal += performance.now() - state.revealStartedAt;
      state.revealStartedAt = null;
    }
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
