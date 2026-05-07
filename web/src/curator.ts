// Corpus curator UI — Stream / Curate / Threshold.
//
// Reachable via a tab bar from the welcome screen. Phone-first; designed to
// work on Galaxy Z Fold 7 cover (904×2316 portrait) and inner display
// (1968×2184 unfolded). Layouts adapt to viewport width breakpoints.

import {
  getCuratorId,
  getProgress,
  postDecision,
  postThreshold,
  streamNext,
  type Candidate,
  type DecisionGroups,
  type LicensePolicy,
  type Suggestion,
} from './curator-api';
import { ANCHOR_QS, disposeSnapshots, encodeAtQ, preEncodeAnchors, type EncodedSnapshot } from './curator-encoder';

type Screen = 'stream' | 'curate' | 'threshold';

export interface CuratorState {
  screen: Screen;
  candidate: Candidate | null;
  license: LicensePolicy | null;
  suggestion: Suggestion | null;
  decision_id: number | null;
  selectedGroups: DecisionGroups;
  selectedSizes: Set<number>;
}

export function startCurator(root: HTMLElement, onExit: () => void): void {
  const curatorId = getCuratorId();
  const state: CuratorState = {
    screen: 'stream',
    candidate: null,
    license: null,
    suggestion: null,
    decision_id: null,
    selectedGroups: {},
    selectedSizes: new Set(),
  };

  const renderStream = async () => {
    state.screen = 'stream';
    state.candidate = null;
    root.innerHTML = `
      <div class="curator-screen curator-stream" data-screen="stream">
        ${renderHeader('Curator — stream')}
        <div class="curator-viewport" id="cv">
          <div class="muted">Loading next candidate…</div>
        </div>
        <div id="meta" class="curator-meta" aria-live="polite"></div>
        <div id="actions" class="curator-actions">
          <button class="curator-skip" id="reject" aria-label="Reject (swipe left)">Skip</button>
          <button class="curator-take primary" id="take" aria-label="Take (swipe right)">Take</button>
        </div>
      </div>
    `;
    bindNav(root, onExit);
    const viewport = root.querySelector<HTMLDivElement>('#cv')!;
    const meta = root.querySelector<HTMLDivElement>('#meta')!;
    try {
      const resp = await streamNext(curatorId);
      if (!resp.candidate) {
        viewport.innerHTML = `<div class="curator-empty"><h2>All decided</h2><p class="muted">${resp.total} candidate(s) reviewed.</p></div>`;
        meta.innerHTML = '';
        root.querySelectorAll<HTMLButtonElement>('#take, #reject').forEach((b) => (b.disabled = true));
        return;
      }
      state.candidate = resp.candidate;
      state.license = resp.license;
      state.suggestion = resp.suggestion;
      state.selectedGroups = {};
      state.selectedSizes = new Set(resp.suggestion?.sizes ?? []);
      for (const g of resp.suggestion?.groups ?? []) {
        (state.selectedGroups as Record<string, boolean>)[g] = true;
      }
      renderStreamImage(viewport, resp.candidate);
      meta.innerHTML = renderCandidateMeta(resp.candidate, resp.license, resp.remaining, resp.total);
      installSwipe(viewport, () => decide('take'), () => decide('reject'));
    } catch (e) {
      viewport.innerHTML = `<div class="curator-empty"><h2>Stream error</h2><p class="muted">${escapeHtml(String((e as Error).message))}</p><p class="muted">POST a manifest to <code>/api/curator/manifest</code> first.</p></div>`;
    }
    root.querySelector<HTMLButtonElement>('#take')?.addEventListener('click', () => decide('take'));
    root.querySelector<HTMLButtonElement>('#reject')?.addEventListener('click', () => decide('reject'));
    document.addEventListener('keydown', keyHandler);
  };

  const keyHandler = (e: KeyboardEvent) => {
    if (state.screen !== 'stream' || !state.candidate) return;
    if (e.key === 'ArrowRight' || e.key === 'f') void decide('take');
    if (e.key === 'ArrowLeft' || e.key === 's') void decide('reject');
  };

  const decide = async (kind: 'take' | 'reject' | 'flag') => {
    if (!state.candidate) return;
    const cand = state.candidate;
    if (kind === 'reject' || kind === 'flag') {
      try {
        await postDecision({
          source_sha256: cand.sha256,
          curator_id: curatorId,
          decision: kind,
          decision_dpr: window.devicePixelRatio,
          decision_viewport_w: window.innerWidth,
          decision_viewport_h: window.innerHeight,
        });
      } catch (e) {
        console.warn('decision failed', e);
      }
      void renderStream();
      return;
    }
    document.removeEventListener('keydown', keyHandler);
    void renderCurate();
  };

  const renderCurate = async () => {
    state.screen = 'curate';
    if (!state.candidate || !state.license) return;
    const cand = state.candidate;
    const sug = state.suggestion;
    const sizeChips = (sug?.sizes ?? [64, 128, 256, 384, 512, 768, 1024, 1536]).slice();
    // Always show all eight, mark unsafe ones disabled.
    const allChips = [64, 128, 256, 384, 512, 768, 1024, 1536];
    root.innerHTML = `
      <div class="curator-screen curator-curate" data-screen="curate">
        ${renderHeader('Curator — review')}
        <div class="curator-preview"><img src="${escapeAttr(cand.blob_url)}" alt="" decoding="async"></div>
        <div class="curator-meta">${renderCandidateMeta(cand, state.license, undefined, undefined)}</div>
        <h2 class="curator-section">Groups</h2>
        <div class="curator-groups" id="groups" role="grid">
          ${groupCell('core_zensim', 'core × zensim')}
          ${groupCell('core_encoding', 'core × encoding')}
          ${groupCell('medium_zensim', 'medium × zensim')}
          ${groupCell('medium_encoding', 'medium × encoding')}
          ${groupCell('full_zensim', 'full × zensim')}
          ${groupCell('full_encoding', 'full × encoding')}
        </div>
        <h2 class="curator-section">Size variants</h2>
        <div class="curator-chips" id="sizes" role="group">
          ${allChips
            .map((d) => {
              const enabled = sizeChips.includes(d);
              const checked = state.selectedSizes.has(d);
              return `<button class="curator-chip ${checked ? 'on' : ''}" data-size="${d}" ${enabled ? '' : 'disabled aria-disabled="true"'}>${d}</button>`;
            })
            .join('')}
        </div>
        <div class="curator-action-row">
          <button id="back">Back</button>
          <button id="save-no-thr" >Save</button>
          <button id="find-thr" class="primary">Find threshold</button>
        </div>
      </div>
    `;
    bindNav(root, onExit);
    // Group toggles
    root.querySelectorAll<HTMLButtonElement>('.curator-group-btn').forEach((btn) => {
      const g = btn.dataset.group as keyof DecisionGroups;
      if ((state.selectedGroups as Record<string, boolean>)[g]) btn.classList.add('on');
      btn.addEventListener('click', () => {
        const cur = (state.selectedGroups as Record<string, boolean>)[g];
        (state.selectedGroups as Record<string, boolean>)[g] = !cur;
        btn.classList.toggle('on', !cur);
      });
    });
    root.querySelectorAll<HTMLButtonElement>('.curator-chip').forEach((btn) => {
      btn.addEventListener('click', () => {
        if (btn.disabled) return;
        const d = Number(btn.dataset.size);
        if (state.selectedSizes.has(d)) {
          state.selectedSizes.delete(d);
          btn.classList.remove('on');
        } else {
          state.selectedSizes.add(d);
          btn.classList.add('on');
        }
      });
    });
    root.querySelector<HTMLButtonElement>('#back')?.addEventListener('click', () => void renderStream());
    root.querySelector<HTMLButtonElement>('#save-no-thr')?.addEventListener('click', async () => {
      const id = await saveDecision();
      if (id != null) {
        state.decision_id = id;
        void renderStream();
      }
    });
    root.querySelector<HTMLButtonElement>('#find-thr')?.addEventListener('click', async () => {
      const id = await saveDecision();
      if (id != null) {
        state.decision_id = id;
        void renderThreshold();
      }
    });
  };

  const saveDecision = async (): Promise<number | null> => {
    if (!state.candidate) return null;
    try {
      const resp = await postDecision({
        source_sha256: state.candidate.sha256,
        curator_id: curatorId,
        decision: 'take',
        groups: state.selectedGroups,
        sizes: [...state.selectedSizes].sort((a, b) => a - b),
        source_q_detected: null,
        recommended_max_dim: state.suggestion?.recommended_max_dim ?? null,
        decision_dpr: window.devicePixelRatio,
        decision_viewport_w: window.innerWidth,
        decision_viewport_h: window.innerHeight,
      });
      return resp.decision_id;
    } catch (e) {
      console.warn('saveDecision failed', e);
      alert('Could not save decision: ' + (e as Error).message);
      return null;
    }
  };

  const renderThreshold = async () => {
    state.screen = 'threshold';
    if (!state.candidate || state.decision_id == null) return;
    const cand = state.candidate;
    const sortedSizes = [...state.selectedSizes].sort((a, b) => b - a);
    const target = sortedSizes[0] ?? 1024;
    root.innerHTML = `
      <div class="curator-screen curator-threshold" data-screen="threshold">
        ${renderHeader('Curator — find threshold')}
        <div class="curator-threshold-info">target ${target}px · jpegli stand-in: browser canvas</div>
        <div class="curator-split" id="split">
          <canvas id="left" aria-label="encoded at 1:1 device pixels"></canvas>
          <canvas id="right" aria-label="encoded at 1:1 CSS pixels (downscaled by DPR)"></canvas>
        </div>
        <div class="curator-slider">
          <input type="range" min="20" max="98" step="1" value="80" id="qslider" aria-label="JPEG quality">
          <div class="curator-q-readout"><span id="qval">q = 80</span></div>
        </div>
        <div class="curator-action-row">
          <button id="back">Back</button>
          <button id="save-thr" class="primary">Save threshold</button>
        </div>
      </div>
    `;
    bindNav(root, onExit);
    const slider = root.querySelector<HTMLInputElement>('#qslider')!;
    const qval = root.querySelector<HTMLSpanElement>('#qval')!;
    const leftC = root.querySelector<HTMLCanvasElement>('#left')!;
    const rightC = root.querySelector<HTMLCanvasElement>('#right')!;

    let snapshots: EncodedSnapshot[] = [];
    let sourceImg: HTMLImageElement | null = null;

    // Wire UI listeners synchronously so the slider readout always tracks the
    // value, even before the image and pre-encoded anchors finish loading.
    slider.addEventListener('input', () => {
      qval.textContent = `q = ${slider.value}`;
    });
    let pendingQ: number | null = null;
    let busy = false;
    const draw = async (q: number) => {
      if (!sourceImg) return;
      let snap = snapshots.find((s) => s.q === q);
      if (!snap) {
        try {
          snap = await encodeAtQ(sourceImg, q);
        } catch (e) {
          console.warn('encode failed at q=' + q, e);
          return;
        }
      }
      const img = await loadImage(snap.url);
      paintSplit(leftC, rightC, img);
    };
    const trigger = async () => {
      if (busy) {
        pendingQ = Number(slider.value);
        return;
      }
      busy = true;
      try {
        await draw(Number(slider.value));
      } finally {
        busy = false;
        if (pendingQ != null) {
          const q = pendingQ;
          pendingQ = null;
          await draw(q);
        }
      }
    };
    slider.addEventListener('change', trigger);
    slider.addEventListener('input', () => void trigger());

    root.querySelector<HTMLButtonElement>('#back')?.addEventListener('click', () => {
      disposeSnapshots(snapshots);
      void renderCurate();
    });
    root.querySelector<HTMLButtonElement>('#save-thr')?.addEventListener('click', async () => {
      const q = Number(slider.value);
      try {
        await postThreshold({
          decision_id: state.decision_id!,
          target_max_dim: target,
          q_imperceptible: q,
          measurement_dpr: window.devicePixelRatio,
          measurement_distance_cm: null,
          encoder_label: 'browser-canvas-jpeg',
        });
        disposeSnapshots(snapshots);
        void renderStream();
      } catch (e) {
        alert('Could not save threshold: ' + (e as Error).message);
      }
    });

    // Async: load the image, pre-encode anchors, and render the default panel.
    try {
      sourceImg = await loadImage(cand.blob_url);
      snapshots = await preEncodeAnchors(sourceImg);
      void draw(Number(slider.value));
    } catch (e) {
      console.warn('threshold setup failed', e);
    }
  };

  void renderStream();
}

// ---------- helpers ----------

function renderHeader(title: string): string {
  return `<header class="curator-header">
    <span class="curator-title">${escapeHtml(title)}</span>
    <button class="curator-exit" id="exit" aria-label="Exit curator">×</button>
  </header>`;
}

function bindNav(root: HTMLElement, onExit: () => void): void {
  root.querySelector<HTMLButtonElement>('#exit')?.addEventListener('click', onExit);
}

function renderStreamImage(host: HTMLDivElement, c: Candidate): void {
  const img = document.createElement('img');
  img.alt = '';
  img.decoding = 'async';
  img.src = c.blob_url;
  img.className = 'curator-img';
  host.innerHTML = '';
  host.appendChild(img);
}

function renderCandidateMeta(
  c: Candidate,
  license: LicensePolicy | null,
  remaining?: number,
  total?: number,
): string {
  const lic = license
    ? `<a class="curator-license-badge" href="${escapeAttr(license.terms_url)}" target="_blank" rel="noreferrer noopener" data-license-id="${escapeAttr(license.id)}">${escapeHtml(license.label)}</a>`
    : '';
  const attribution = c.license_url
    ? `<a class="curator-attribution-link" href="${escapeAttr(c.license_url)}" target="_blank" rel="noreferrer noopener">attribution</a>`
    : '';
  const dims = c.width && c.height ? `${c.width}×${c.height}` : '?';
  const sz = c.size_bytes ? `${(c.size_bytes / 1024).toFixed(0)} KB` : '';
  const fmt = c.format ?? '';
  const corpus = c.corpus;
  const remainText = remaining != null && total != null ? `${total - remaining}/${total}` : '';
  return `<div class="curator-meta-row">
    <span class="curator-corpus">${escapeHtml(corpus)}</span>
    <span class="curator-fmt">${escapeHtml(fmt)} · ${dims} · ${sz}</span>
    ${lic}
    ${attribution}
    ${remainText ? `<span class="curator-progress">${remainText}</span>` : ''}
  </div>`;
}

function groupCell(g: keyof DecisionGroups, label: string): string {
  return `<button class="curator-group-btn" data-group="${g}" role="checkbox" aria-checked="false">${escapeHtml(label)}</button>`;
}

function paintSplit(left: HTMLCanvasElement, right: HTMLCanvasElement, img: HTMLImageElement): void {
  const dpr = window.devicePixelRatio || 1;
  const naturalW = img.naturalWidth;
  const naturalH = img.naturalHeight;

  // Container queries: pick a window centered on the image.
  const leftRect = left.getBoundingClientRect();
  const rightRect = right.getBoundingClientRect();

  // Left: 1:1 device pixels — canvas backing = (cssW * dpr), draw image 1:1 device px
  const lcssW = Math.max(1, Math.floor(leftRect.width));
  const lcssH = Math.max(1, Math.floor(leftRect.height));
  left.width = lcssW * dpr;
  left.height = lcssH * dpr;
  const lctx = left.getContext('2d')!;
  lctx.imageSmoothingEnabled = false;
  // Center crop in image coordinates with one image pixel per device pixel.
  const lWindowW = Math.min(naturalW, left.width);
  const lWindowH = Math.min(naturalH, left.height);
  const sx = Math.floor((naturalW - lWindowW) / 2);
  const sy = Math.floor((naturalH - lWindowH) / 2);
  lctx.fillStyle = '#000';
  lctx.fillRect(0, 0, left.width, left.height);
  const ldx = Math.floor((left.width - lWindowW) / 2);
  const ldy = Math.floor((left.height - lWindowH) / 2);
  lctx.drawImage(img, sx, sy, lWindowW, lWindowH, ldx, ldy, lWindowW, lWindowH);

  // Right: 1:1 CSS pixels — same crop window, downscaled by DPR.
  const rcssW = Math.max(1, Math.floor(rightRect.width));
  const rcssH = Math.max(1, Math.floor(rightRect.height));
  right.width = rcssW * dpr;
  right.height = rcssH * dpr;
  const rctx = right.getContext('2d')!;
  rctx.imageSmoothingEnabled = true;
  rctx.imageSmoothingQuality = 'high';
  rctx.fillStyle = '#000';
  rctx.fillRect(0, 0, right.width, right.height);
  const rWindowW = lWindowW;
  const rWindowH = lWindowH;
  const dstW = Math.min(right.width, rWindowW / dpr);
  const dstH = Math.min(right.height, rWindowH / dpr);
  const rdx = Math.floor((right.width - dstW) / 2);
  const rdy = Math.floor((right.height - dstH) / 2);
  rctx.drawImage(img, sx, sy, rWindowW, rWindowH, rdx, rdy, dstW, dstH);
}

function loadImage(url: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.crossOrigin = 'anonymous';
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error(`failed to load ${url}`));
    img.src = url;
  });
}

function installSwipe(host: HTMLElement, onRight: () => void, onLeft: () => void): void {
  let startX = 0;
  let startY = 0;
  let down = false;
  host.addEventListener('pointerdown', (e: PointerEvent) => {
    down = true;
    startX = e.clientX;
    startY = e.clientY;
  });
  host.addEventListener('pointerup', (e: PointerEvent) => {
    if (!down) return;
    down = false;
    const dx = e.clientX - startX;
    const dy = e.clientY - startY;
    if (Math.abs(dx) > 80 && Math.abs(dx) > Math.abs(dy)) {
      if (dx > 0) onRight();
      else onLeft();
    }
  });
  host.addEventListener('pointercancel', () => { down = false; });
}

function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);
}

function escapeAttr(s: string): string {
  return escapeHtml(s);
}

// Re-exported for tests / dev tools.
export { ANCHOR_QS };

// Tab-bar helper used by main.ts.
export interface CuratorTabHandlers {
  onCurator: () => void;
  onRate: () => void;
  onCalibrate: () => void;
  onSuggest: () => void;
}

export function renderTabBar(active: 'rate' | 'curator' | 'calibrate' | 'suggest', _h: CuratorTabHandlers): string {
  const cls = (k: string) => (k === active ? 'on' : '');
  return `<nav class="squintly-tabs" aria-label="Mode">
    <button class="${cls('rate')}" data-tab="rate">Rate</button>
    <button class="${cls('curator')}" data-tab="curator">Curator</button>
    <button class="${cls('suggest')}" data-tab="suggest">Suggest</button>
    <button class="${cls('calibrate')}" data-tab="calibrate">Calibrate</button>
  </nav>`;
}

export function bindTabBar(root: HTMLElement, h: CuratorTabHandlers): void {
  root.querySelectorAll<HTMLButtonElement>('.squintly-tabs button').forEach((b) => {
    b.addEventListener('click', () => {
      const t = b.dataset.tab;
      if (t === 'curator') h.onCurator();
      else if (t === 'rate') h.onRate();
      else if (t === 'calibrate') h.onCalibrate();
      else if (t === 'suggest') h.onSuggest();
    });
  });
}

// Progress summary (used on the welcome screen when curator decisions exist).
export async function renderProgressSummary(): Promise<string> {
  const id = getCuratorId();
  try {
    const p = await getProgress(id);
    if (p.decisions === 0) return '';
    return `<p class="muted curator-progress-summary">Curator: ${p.takes} taken · ${p.rejects} skipped · ${p.thresholds} thresholds across ${p.decisions} decisions of ${p.total_candidates}.</p>`;
  } catch {
    return '';
  }
}
