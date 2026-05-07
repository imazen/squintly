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
}

export interface TrialController {
  start(): Promise<void>;
  end(): void;
}

export function startTrials(root: HTMLElement, sessionId: string): TrialController {
  let aborted = false;
  let trialCount = 0;

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
          <div class="reveal-hint" id="hint">${isPair ? 'tap A or B' : 'hold to compare with original'}</div>
        </div>
        <div id="panel"></div>
      </div>
    `;
    const viewport = root.querySelector<HTMLDivElement>('#viewport')!;
    const img = root.querySelector<HTMLImageElement>('#stimulus')!;
    const panel = root.querySelector<HTMLDivElement>('#panel')!;
    const hint = root.querySelector<HTMLDivElement>('#hint')!;
    const menu = root.querySelector<HTMLButtonElement>('#menu')!;
    menu.addEventListener('click', () => openMenu());

    // Display the encoding-under-test (or A for pair) at intrinsic 1:1 device-px size
    // when feasible; otherwise scale-to-fit to viewport while preserving aspect.
    const dpr = window.devicePixelRatio ?? 1;
    let currentSrc: 'a' | 'b' | 'ref' = 'a';
    const setSrc = (which: 'a' | 'b' | 'ref') => {
      currentSrc = which;
      img.src = which === 'ref' ? trial.source_url : which === 'a' ? trial.a.url : trial.b!.url;
    };
    img.addEventListener('load', () => {
      const w = img.naturalWidth;
      const h = img.naturalHeight;
      // Intrinsic-to-device target: 1.0 → CSS px = intrinsic / dpr.
      // Cap at the viewport dimensions.
      const rect = viewport.getBoundingClientRect();
      const targetCssW = w / dpr;
      const targetCssH = h / dpr;
      const scale = Math.min(1, rect.width / targetCssW, rect.height / targetCssH);
      img.style.width = `${targetCssW * scale}px`;
      img.style.height = `${targetCssH * scale}px`;
      if (state.shownAt === 0) state.shownAt = performance.now();
    });
    setSrc('a');

    // Hold-to-reveal: while held, show reference. On release, show the encoding.
    const startReveal = () => {
      if (currentSrc === 'ref') return;
      state.revealStartedAt = performance.now();
      state.revealCount += 1;
      viewport.classList.add('revealing');
      hint.textContent = 'showing original';
      setSrc('ref');
    };
    const endReveal = () => {
      if (state.revealStartedAt !== null) {
        state.revealMsTotal += performance.now() - state.revealStartedAt;
        state.revealStartedAt = null;
      }
      viewport.classList.remove('revealing');
      hint.textContent = isPair ? 'tap A or B' : 'hold to compare with original';
      setSrc('a');
    };
    // For pair trials, the hold gesture toggles A↔B instead. (Reference is implicit;
    // observers compare A and B for closeness to the source — we still want them to
    // *see* the reference, so a tap-to-reveal short press is added to the menu.)
    if (!isPair) {
      viewport.addEventListener('pointerdown', startReveal);
      viewport.addEventListener('pointerup', endReveal);
      viewport.addEventListener('pointercancel', endReveal);
      viewport.addEventListener('pointerleave', endReveal);
    } else {
      let isB = false;
      viewport.addEventListener('pointerdown', () => {
        isB = !isB;
        setSrc(isB ? 'b' : 'a');
        hint.textContent = isB ? 'B' : 'A';
      });
    }

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
        submit(choice, state, trial, img);
      });
    });
  };

  const submit = async (
    choice: string,
    state: TrialState,
    trial: TrialPayload,
    img: HTMLImageElement,
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
