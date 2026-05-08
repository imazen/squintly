// Corpus curator UI — Stream / Curate / Threshold.
//
// Reachable via a tab bar from the welcome screen. Phone-first; designed to
// work on Galaxy Z Fold 7 cover (904×2316 portrait) and inner display
// (1968×2184 unfolded). Layouts adapt to viewport width breakpoints.

import {
  generateVariant,
  getCuratorId,
  getProgress,
  listLicenses,
  postDecision,
  postThreshold,
  streamNext,
  undoDecision,
  type BppGate,
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
  bppGate: BppGate | null;
  decision_id: number | null;
  selectedGroups: DecisionGroups;
  selectedSizes: Set<number>;
}

interface CuratorPrefs {
  corpus: string[];
  license_id: string[];
}

const PREFS_KEY = 'squintly:curator_prefs';

function loadPrefs(): CuratorPrefs {
  try {
    const raw = localStorage.getItem(PREFS_KEY);
    if (!raw) return { corpus: [], license_id: [] };
    const obj = JSON.parse(raw);
    return {
      corpus: Array.isArray(obj.corpus) ? obj.corpus.filter((s: unknown) => typeof s === 'string') : [],
      license_id: Array.isArray(obj.license_id) ? obj.license_id.filter((s: unknown) => typeof s === 'string') : [],
    };
  } catch {
    return { corpus: [], license_id: [] };
  }
}

function savePrefs(p: CuratorPrefs): void {
  try {
    localStorage.setItem(PREFS_KEY, JSON.stringify(p));
  } catch {
    // localStorage may be unavailable in private mode; non-fatal.
  }
}

export function startCurator(root: HTMLElement, onExit: () => void): void {
  const curatorId = getCuratorId();
  const undoStack: string[] = [];
  let prefs = loadPrefs();
  const state: CuratorState = {
    screen: 'stream',
    candidate: null,
    license: null,
    suggestion: null,
    bppGate: null,
    decision_id: null,
    selectedGroups: {},
    selectedSizes: new Set(),
  };

  const renderStream = async () => {
    state.screen = 'stream';
    state.candidate = null;
    const undoCount = undoStack.length;
    root.innerHTML = `
      <div class="curator-screen curator-stream" data-screen="stream">
        ${renderHeader('Curator — stream')}
        <div class="curator-status-row">
          <span id="status-i-of-n" class="muted"></span>
          <span id="filter-pill" class="curator-filter-pill" hidden></span>
          <button id="undo" class="curator-undo" ${undoCount === 0 ? 'disabled' : ''} title="Undo last decision (u or z)">↶ Undo${undoCount ? ` (${undoCount})` : ''}</button>
          <button id="settings" class="curator-settings-btn" aria-label="Filter settings" title="Filter settings">⚙</button>
        </div>
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
    root.querySelector<HTMLButtonElement>('#undo')?.addEventListener('click', () => void doUndo());
    root.querySelector<HTMLButtonElement>('#settings')?.addEventListener('click', () => void openSettings());
    updateFilterPill();
    const viewport = root.querySelector<HTMLDivElement>('#cv')!;
    const meta = root.querySelector<HTMLDivElement>('#meta')!;
    try {
      const resp = await streamNext(curatorId, {
        corpus: prefs.corpus,
        license_id: prefs.license_id,
      });
      if (!resp.candidate) {
        viewport.innerHTML = `<div class="curator-empty"><h2>All decided</h2><p class="muted">${resp.total} candidate(s) reviewed.</p></div>`;
        meta.innerHTML = '';
        root.querySelectorAll<HTMLButtonElement>('#take, #reject').forEach((b) => (b.disabled = true));
        return;
      }
      state.candidate = resp.candidate;
      state.license = resp.license;
      state.suggestion = resp.suggestion;
      state.bppGate = resp.bpp_gate;
      state.selectedGroups = {};
      state.selectedSizes = new Set(resp.suggestion?.sizes ?? []);
      for (const g of resp.suggestion?.groups ?? []) {
        (state.selectedGroups as Record<string, boolean>)[g] = true;
      }
      renderStreamImage(viewport, resp.candidate);
      meta.innerHTML = renderCandidateMeta(resp.candidate, resp.license, resp.remaining, resp.total)
        + renderBppGate(resp.bpp_gate);
      const status = root.querySelector<HTMLSpanElement>('#status-i-of-n');
      if (status) {
        const decided = resp.total - resp.remaining;
        status.textContent = `${decided} / ${resp.total}`;
      }
      void prefetchNext();
      installSwipe(viewport, {
        onRight: () => decide('take'),
        onLeft: () => decide('reject'),
        onDown: () => promptFlag(),
        onPeekStart: () => showPeekOverlay(viewport, state.candidate, state.bppGate, state.license),
        onPeekEnd: () => hidePeekOverlay(viewport),
      });
    } catch (e) {
      viewport.innerHTML = `<div class="curator-empty"><h2>Stream error</h2><p class="muted">${escapeHtml(String((e as Error).message))}</p><p class="muted">POST a manifest to <code>/api/curator/manifest</code> first.</p></div>`;
    }
    root.querySelector<HTMLButtonElement>('#take')?.addEventListener('click', () => decide('take'));
    root.querySelector<HTMLButtonElement>('#reject')?.addEventListener('click', () => decide('reject'));
    document.addEventListener('keydown', keyHandler);
  };

  const keyHandler = (e: KeyboardEvent) => {
    if (state.screen !== 'stream') return;
    if (state.candidate && (e.key === 'ArrowRight' || e.key === 'f')) {
      void decide('take');
    } else if (state.candidate && (e.key === 'ArrowLeft' || e.key === 's')) {
      void decide('reject');
    } else if (e.key === 'u' || e.key === 'z') {
      void doUndo();
    }
  };

  const doUndo = async () => {
    const sha = undoStack.pop();
    try {
      const resp = await undoDecision(curatorId, sha);
      if (!resp.undone) return;
    } catch (e) {
      console.warn('undo failed', e);
      // Restore the stack so the user can retry.
      if (sha) undoStack.push(sha);
      return;
    }
    void renderStream();
  };

  const updateFilterPill = () => {
    const pill = root.querySelector<HTMLSpanElement>('#filter-pill');
    if (!pill) return;
    const parts: string[] = [];
    if (prefs.corpus.length > 0) parts.push(`corpus: ${prefs.corpus.length}`);
    if (prefs.license_id.length > 0) parts.push(`license: ${prefs.license_id.length}`);
    if (parts.length === 0) {
      pill.hidden = true;
      pill.textContent = '';
    } else {
      pill.hidden = false;
      pill.textContent = `filter ${parts.join(' · ')}`;
    }
  };

  const openSettings = async () => {
    const [progress, licenses] = await Promise.all([
      getProgress(curatorId).catch(() => null),
      listLicenses().catch(() => [] as LicensePolicy[]),
    ]);
    const corpora = (progress?.by_corpus ?? []).slice().sort((a, b) => a.corpus.localeCompare(b.corpus));
    const scrim = document.createElement('div');
    scrim.className = 'scrim';
    scrim.innerHTML = `
      <div class="card curator-settings-card">
        <h2>Filter the stream</h2>
        <p class="muted">Restrict which candidates the curator sees. Empty = no filter (default).</p>
        <h3>By corpus</h3>
        <div class="curator-filter-list" id="cf-corpus">
          ${corpora.length === 0 ? '<p class="muted">No corpora loaded yet.</p>' : corpora.map((c) => {
            const checked = prefs.corpus.includes(c.corpus) ? 'checked' : '';
            const remaining = c.total - c.decided;
            return `<label class="curator-filter-row">
              <input type="checkbox" data-corpus="${escapeAttr(c.corpus)}" ${checked}>
              <span class="curator-filter-name">${escapeHtml(c.corpus)}</span>
              <span class="muted curator-filter-count">${remaining} left of ${c.total}</span>
            </label>`;
          }).join('')}
        </div>
        <h3>By license</h3>
        <div class="curator-filter-list" id="cf-license">
          ${licenses.length === 0 ? '<p class="muted">License registry unavailable.</p>' : licenses.map((p) => {
            const checked = prefs.license_id.includes(p.id) ? 'checked' : '';
            return `<label class="curator-filter-row">
              <input type="checkbox" data-license="${escapeAttr(p.id)}" ${checked}>
              <span class="curator-filter-name">${escapeHtml(p.label)}</span>
              <span class="muted curator-filter-count">${p.redistribute_bytes ? '' : 'research-only'}</span>
            </label>`;
          }).join('')}
        </div>
        <div class="choice-row" style="margin-top:12px;">
          <button id="cf-clear">Clear filter</button>
          <button id="cf-cancel">Cancel</button>
          <button id="cf-apply" class="primary">Apply</button>
        </div>
      </div>
    `;
    document.body.appendChild(scrim);
    scrim.querySelector<HTMLButtonElement>('#cf-cancel')?.addEventListener('click', () => scrim.remove());
    scrim.querySelector<HTMLButtonElement>('#cf-clear')?.addEventListener('click', () => {
      prefs = { corpus: [], license_id: [] };
      savePrefs(prefs);
      scrim.remove();
      void renderStream();
    });
    scrim.querySelector<HTMLButtonElement>('#cf-apply')?.addEventListener('click', () => {
      const corpusSel: string[] = [];
      scrim
        .querySelectorAll<HTMLInputElement>('#cf-corpus input[type="checkbox"]:checked')
        .forEach((b) => {
          const c = b.dataset.corpus;
          if (c) corpusSel.push(c);
        });
      const licenseSel: string[] = [];
      scrim
        .querySelectorAll<HTMLInputElement>('#cf-license input[type="checkbox"]:checked')
        .forEach((b) => {
          const c = b.dataset.license;
          if (c) licenseSel.push(c);
        });
      prefs = { corpus: corpusSel, license_id: licenseSel };
      savePrefs(prefs);
      scrim.remove();
      void renderStream();
    });
  };

  const decide = async (kind: 'take' | 'reject' | 'flag', rejectReason?: string) => {
    if (!state.candidate) return;
    const cand = state.candidate;
    if (kind === 'reject' || kind === 'flag') {
      try {
        await postDecision({
          source_sha256: cand.sha256,
          curator_id: curatorId,
          decision: kind,
          reject_reason: rejectReason ?? null,
          decision_dpr: window.devicePixelRatio,
          decision_viewport_w: window.innerWidth,
          decision_viewport_h: window.innerHeight,
        });
        undoStack.push(cand.sha256);
        if (undoStack.length > 50) undoStack.shift();
      } catch (e) {
        console.warn('decision failed', e);
      }
      void renderStream();
      return;
    }
    // For 'take' the actual decision write happens inside renderCurate via
    // saveDecision(). Stack push happens there too so an undo from curate
    // works as expected.
    document.removeEventListener('keydown', keyHandler);
    void renderCurate();
  };

  const promptFlag = () => {
    const reasons: { id: string; label: string }[] = [
      { id: 'low_quality', label: 'Source quality too low' },
      { id: 'inappropriate', label: 'Inappropriate / unsafe' },
      { id: 'duplicate', label: 'Looks duplicated' },
      { id: 'broken', label: "Won't load / broken" },
      { id: 'license_concern', label: 'License concern' },
      { id: 'other', label: 'Other (skip without flag)' },
    ];
    const scrim = document.createElement('div');
    scrim.className = 'scrim';
    scrim.innerHTML = `
      <div class="card curator-flag-card">
        <h2>Flag this image</h2>
        <p class="muted">Records the reason; the candidate is removed from your stream.</p>
        <div class="curator-flag-list">
          ${reasons.map((r) => `<button data-id="${r.id}">${escapeHtml(r.label)}</button>`).join('')}
        </div>
        <div class="choice-row" style="margin-top:8px;">
          <button id="flag-cancel">Cancel</button>
        </div>
      </div>
    `;
    document.body.appendChild(scrim);
    scrim.querySelectorAll<HTMLButtonElement>('.curator-flag-list button').forEach((b) => {
      b.addEventListener('click', () => {
        const reason = b.dataset.id ?? 'other';
        scrim.remove();
        if (reason === 'other') {
          void decide('reject');
        } else {
          void decide('flag', reason);
        }
      });
    });
    scrim.querySelector<HTMLButtonElement>('#flag-cancel')?.addEventListener('click', () => scrim.remove());
  };

  // Single-image lookahead: as soon as a candidate is shown, prefetch the
  // *next* candidate's bytes via a hidden <img> element so the next render
  // is instant. Phone bandwidth is the constraint, not RAM (per spec §2.1).
  let lookaheadEl: HTMLImageElement | null = null;
  const prefetchNext = async () => {
    try {
      const probe = await streamNext(curatorId, {
        skip: 1,
        corpus: prefs.corpus,
        license_id: prefs.license_id,
      });
      const next = probe.candidate;
      if (!next) return;
      if (lookaheadEl) lookaheadEl.remove();
      lookaheadEl = new Image();
      lookaheadEl.decoding = 'async';
      lookaheadEl.style.display = 'none';
      lookaheadEl.src = next.blob_url;
      document.body.appendChild(lookaheadEl);
    } catch {
      // ignore prefetch failures
    }
  };

  const renderCurate = async () => {
    state.screen = 'curate';
    if (!state.candidate || !state.license) return;
    const cand = state.candidate;
    const sug = state.suggestion;
    // Always show all eight chips. A chip is disabled iff the suggestion's
    // sizes array exists, is non-empty, AND doesn't include this dim — that
    // means the backend computed an upper bound and this chip would upscale.
    // When dims are unknown the suggestion returns all 8, and even if it's
    // empty (legacy responses) we treat that as "no info, let curator pick."
    const allChips = [64, 128, 256, 384, 512, 768, 1024, 1536];
    const safeSet = sug && sug.sizes.length > 0 ? new Set(sug.sizes) : new Set(allChips);
    root.innerHTML = `
      <div class="curator-screen curator-curate" data-screen="curate">
        ${renderHeader('Curator — review')}
        <div class="curator-preview"><img src="${escapeAttr(cand.blob_url)}" alt="" decoding="async"></div>
        <div class="curator-meta">${renderCandidateMeta(cand, state.license, undefined, undefined)}${renderBppGate(state.bppGate)}</div>
        <h2 class="curator-section">Groups</h2>
        <div class="curator-groups" id="groups" role="grid">
          ${groupCell('core_zensim', 'core × zensim')}
          ${groupCell('core_encoding', 'core × encoding')}
          ${groupCell('medium_zensim', 'medium × zensim')}
          ${groupCell('medium_encoding', 'medium × encoding')}
          ${groupCell('full_zensim', 'full × zensim')}
          ${groupCell('full_encoding', 'full × encoding')}
        </div>
        <h2 class="curator-section">Downsamples to allow</h2>
        <p class="muted curator-chip-help">Tap to toggle each target max-dim. Greyed-out chips would upscale the source.</p>
        <div class="curator-chips" id="sizes" role="group">
          ${allChips
            .map((d) => {
              const enabled = safeSet.has(d);
              const checked = state.selectedSizes.has(d);
              return `<button class="curator-chip ${checked ? 'on' : ''}" data-size="${d}" ${enabled ? '' : 'disabled aria-disabled="true"'}>${d}</button>`;
            })
            .join('')}
        </div>
        <details class="curator-preview-strip-wrap" id="preview-wrap">
          <summary class="muted">Preview at selected sizes</summary>
          <div class="curator-preview-strip" id="preview-strip" data-empty="true">
            <p class="muted">Select sizes above, then expand to see thumbnails.</p>
          </div>
        </details>
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
        if (previewWrap?.open) void renderPreviewStrip();
      });
    });

    // Preview strip — renders one thumbnail per selected size by drawing the
    // source onto a sized canvas (browser high-quality smoothing — close
    // enough to Mitchell for a sanity check). Lazy: only fires when the
    // <details> is expanded, and re-fires when chip toggles change the set.
    const previewWrap = root.querySelector<HTMLDetailsElement>('#preview-wrap');
    const renderPreviewStrip = async () => {
      const host = root.querySelector<HTMLDivElement>('#preview-strip');
      if (!host || !cand) return;
      const sorted = [...state.selectedSizes].sort((a, b) => a - b);
      if (sorted.length === 0) {
        host.dataset.empty = 'true';
        host.innerHTML = '<p class="muted">Select sizes above to see thumbnails.</p>';
        return;
      }
      host.dataset.empty = 'false';
      host.innerHTML = sorted
        .map((d) => `<figure class="curator-thumb"><canvas data-size="${d}"></canvas><figcaption>${d}px</figcaption></figure>`)
        .join('');
      let img: HTMLImageElement;
      try {
        img = await loadImage(cand.blob_url);
      } catch (e) {
        host.innerHTML = `<p class="muted">Couldn't load source: ${escapeHtml(String((e as Error).message))}</p>`;
        return;
      }
      const naturalMax = Math.max(img.naturalWidth, img.naturalHeight) || 1;
      host.querySelectorAll<HTMLCanvasElement>('canvas[data-size]').forEach((c) => {
        const target = Number(c.dataset.size) || 0;
        const scale = Math.min(1, target / naturalMax);
        const w = Math.max(1, Math.round(img.naturalWidth * scale));
        const h = Math.max(1, Math.round(img.naturalHeight * scale));
        c.width = w;
        c.height = h;
        c.style.maxWidth = `${Math.min(w, 160)}px`;
        c.style.height = 'auto';
        const ctx = c.getContext('2d');
        if (!ctx) return;
        ctx.imageSmoothingEnabled = true;
        ctx.imageSmoothingQuality = 'high';
        ctx.drawImage(img, 0, 0, w, h);
      });
    };
    previewWrap?.addEventListener('toggle', () => {
      if (previewWrap.open) void renderPreviewStrip();
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
      undoStack.push(state.candidate.sha256);
      if (undoStack.length > 50) undoStack.shift();
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
    // Tick marks at the pre-encoded anchor q values so the slider visually
    // confirms which positions have a snapshot.
    const ticksDatalistId = 'curator-q-ticks';
    const ticksHtml = ANCHOR_QS.map((q) => `<option value="${q}"></option>`).join('');
    root.innerHTML = `
      <div class="curator-screen curator-threshold" data-screen="threshold">
        ${renderHeader('Curator — find threshold')}
        <div class="curator-threshold-info">target ${target}px · encoder: browser-canvas-jpeg</div>
        <div class="curator-split" id="split" data-mode="encoded">
          <canvas id="left" aria-label="encoded at 1:1 device pixels"></canvas>
          <canvas id="right" aria-label="encoded at 1:1 CSS pixels (downscaled by DPR)"></canvas>
        </div>
        <div class="curator-slider">
          <input type="range" min="20" max="98" step="1" value="80" id="qslider" list="${ticksDatalistId}" aria-label="JPEG quality">
          <datalist id="${ticksDatalistId}">${ticksHtml}</datalist>
          <div class="curator-q-readout">
            <button class="curator-q-nudge" id="q-down" aria-label="Lower q by 1">−1</button>
            <span id="qval">q = 80</span>
            <button class="curator-q-nudge" id="q-up" aria-label="Raise q by 1">+1</button>
          </div>
        </div>
        <div class="curator-toggle-row">
          <button id="toggle-ref" aria-pressed="false" title="Compare against the uncompressed source">Show reference</button>
          <button id="reset-crop" title="Recenter the crop window">⊕ Reset crop</button>
        </div>
        <div class="curator-action-row">
          <button id="back">Back</button>
          <button id="gen-variant" title="Generate the variant for this size at the saved q">Generate variant</button>
          <button id="save-thr" class="primary">Save threshold</button>
        </div>
        <div id="gen-status" class="curator-gen-status muted" hidden></div>
      </div>
    `;
    bindNav(root, onExit);
    const slider = root.querySelector<HTMLInputElement>('#qslider')!;
    const qval = root.querySelector<HTMLSpanElement>('#qval')!;
    const leftC = root.querySelector<HTMLCanvasElement>('#left')!;
    const rightC = root.querySelector<HTMLCanvasElement>('#right')!;
    const splitEl = root.querySelector<HTMLDivElement>('#split')!;
    const toggleRef = root.querySelector<HTMLButtonElement>('#toggle-ref')!;
    const qDown = root.querySelector<HTMLButtonElement>('#q-down')!;
    const qUp = root.querySelector<HTMLButtonElement>('#q-up')!;

    let snapshots: EncodedSnapshot[] = [];
    let sourceImg: HTMLImageElement | null = null;
    let showingReference = false;
    // Crop offset (image-pixels relative to center). Updated by drag-to-pan
    // on either canvas; both panels stay aligned via a shared offset.
    const cropOffset = { x: 0, y: 0 };

    const setQValReadout = () => {
      qval.textContent = `q = ${slider.value}`;
    };
    setQValReadout();
    slider.addEventListener('input', setQValReadout);

    const nudge = (delta: number) => {
      const next = Math.max(
        Number(slider.min),
        Math.min(Number(slider.max), Number(slider.value) + delta),
      );
      if (next === Number(slider.value)) return;
      slider.value = String(next);
      setQValReadout();
      void trigger();
    };
    qDown.addEventListener('click', () => nudge(-1));
    qUp.addEventListener('click', () => nudge(+1));

    let pendingQ: number | null = null;
    let busy = false;
    let lastEncodedImg: HTMLImageElement | null = null;
    const drawEncoded = async (q: number) => {
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
      lastEncodedImg = img;
      paintSplit(leftC, rightC, img, cropOffset);
    };
    const drawReference = () => {
      if (!sourceImg) return;
      paintSplit(leftC, rightC, sourceImg, cropOffset);
    };
    // Re-paint without re-encoding — used while dragging the crop. Picks the
    // current image (encoded or reference) and just translates the offset.
    const repaintCurrent = () => {
      if (showingReference) {
        if (sourceImg) paintSplit(leftC, rightC, sourceImg, cropOffset);
      } else if (lastEncodedImg) {
        paintSplit(leftC, rightC, lastEncodedImg, cropOffset);
      }
    };
    const draw = async (q: number) => {
      if (showingReference) {
        drawReference();
      } else {
        await drawEncoded(q);
      }
    };
    // Snap a slider value to the nearest pre-encoded anchor — used while
    // the curator is mid-drag so we don't need to JIT encode every tick.
    const nearestAnchor = (q: number): number => {
      let best = ANCHOR_QS[0];
      let bestD = Math.abs(q - best);
      for (const a of ANCHOR_QS) {
        const d = Math.abs(q - a);
        if (d < bestD) {
          bestD = d;
          best = a;
        }
      }
      return best;
    };

    const HOVER_PAUSE_MS = 80; // spec §2.3
    let hoverTimer: number | null = null;
    let dragging = false;

    // Immediate path: paint the nearest pre-encoded anchor. Used during drag
    // so the panels stay responsive without burning the encoder.
    const drawAnchorSnap = () => {
      if (showingReference) {
        drawReference();
        return;
      }
      const targetQ = nearestAnchor(Number(slider.value));
      // Run drawEncoded but it'll hit the snapshot cache (no JIT encode).
      void drawEncoded(targetQ);
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

    // Debounced JIT encode after slider stops moving. Schedules a `trigger()`
    // call HOVER_PAUSE_MS after the last input event. Cancels itself on each
    // new input so a continuous drag never queues encodes.
    const scheduleJit = () => {
      if (hoverTimer != null) {
        window.clearTimeout(hoverTimer);
      }
      hoverTimer = window.setTimeout(() => {
        hoverTimer = null;
        void trigger();
      }, HOVER_PAUSE_MS);
    };

    // While dragging: paint the nearest anchor immediately (instant feedback,
    // no encode cost), then schedule a JIT encode at the actual q value once
    // the curator pauses. On `change` (drag end / keyboard nudge), encode now.
    slider.addEventListener('input', () => {
      if (dragging) drawAnchorSnap();
      scheduleJit();
    });
    slider.addEventListener('change', () => {
      if (hoverTimer != null) {
        window.clearTimeout(hoverTimer);
        hoverTimer = null;
      }
      void trigger();
    });

    // On pointerup the spec asks for both panels to swap to the uncompressed
    // source so the curator has a fixed reference for the saved threshold.
    // We do that here, then leave the toggle-ref button in the on state so
    // the curator can flip back to the encoded view at will.
    const setReferenceMode = (on: boolean) => {
      showingReference = on;
      toggleRef.setAttribute('aria-pressed', String(on));
      toggleRef.textContent = on ? 'Showing reference' : 'Show reference';
      splitEl.dataset.mode = on ? 'reference' : 'encoded';
      void trigger();
    };
    slider.addEventListener('pointerup', () => {
      dragging = false;
      setReferenceMode(true);
    });
    slider.addEventListener('touchend', () => {
      dragging = false;
      setReferenceMode(true);
    });
    slider.addEventListener('pointerdown', () => {
      dragging = true;
      setReferenceMode(false);
    });
    slider.addEventListener('touchstart', () => {
      dragging = true;
      setReferenceMode(false);
    }, { passive: true });
    toggleRef.addEventListener('click', () => setReferenceMode(!showingReference));

    // Drag-to-pan on the split. Touch + mouse via Pointer Events. Shares one
    // offset across both panels so they stay aligned. Delta is in CSS pixels;
    // convert to image-pixel-space by × dpr (left canvas is 1:1 device).
    let panning = false;
    let lastX = 0;
    let lastY = 0;
    let activePointer: number | null = null;
    const dpr = window.devicePixelRatio || 1;
    const onPointerDown = (e: PointerEvent) => {
      if (activePointer != null) return;
      activePointer = e.pointerId;
      panning = true;
      lastX = e.clientX;
      lastY = e.clientY;
      splitEl.setPointerCapture(e.pointerId);
      splitEl.classList.add('panning');
      e.preventDefault();
    };
    const onPointerMove = (e: PointerEvent) => {
      if (!panning || e.pointerId !== activePointer) return;
      const dx = (e.clientX - lastX) * dpr;
      const dy = (e.clientY - lastY) * dpr;
      lastX = e.clientX;
      lastY = e.clientY;
      // Drag direction = move the *content* the way the finger moves, so
      // panning right shows content to the right of center → offset.x decreases.
      cropOffset.x -= dx;
      cropOffset.y -= dy;
      repaintCurrent();
    };
    const onPointerUp = (e: PointerEvent) => {
      if (e.pointerId !== activePointer) return;
      panning = false;
      activePointer = null;
      splitEl.classList.remove('panning');
      try { splitEl.releasePointerCapture(e.pointerId); } catch { /* ignore */ }
    };
    splitEl.addEventListener('pointerdown', onPointerDown);
    splitEl.addEventListener('pointermove', onPointerMove);
    splitEl.addEventListener('pointerup', onPointerUp);
    splitEl.addEventListener('pointercancel', onPointerUp);

    root.querySelector<HTMLButtonElement>('#reset-crop')?.addEventListener('click', () => {
      cropOffset.x = 0;
      cropOffset.y = 0;
      repaintCurrent();
    });

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

    // Generate-variant: kicks off the backend Mitchell-Netravali resize +
    // jpeg-encode + R2-upload pipeline. Uses the slider's current q so the
    // curator can preview a variant at any quality without saving the
    // threshold first.
    const genBtn = root.querySelector<HTMLButtonElement>('#gen-variant');
    const genStatus = root.querySelector<HTMLDivElement>('#gen-status');
    genBtn?.addEventListener('click', async () => {
      if (state.decision_id == null) return;
      const q = Number(slider.value);
      genBtn.disabled = true;
      if (genStatus) {
        genStatus.hidden = false;
        genStatus.textContent = `Generating ${target}px @ q=${q}…`;
      }
      try {
        const resp = await generateVariant({
          decision_id: state.decision_id,
          target_max_dim: target,
          quality: q,
        });
        if (genStatus) {
          const kb = (resp.size_bytes / 1024).toFixed(1);
          genStatus.innerHTML = `Generated <a href="${escapeAttr(resp.generated_url)}" target="_blank" rel="noreferrer noopener">${resp.width}×${resp.height} JPEG</a> (${kb} KB) at q=${resp.source_q}.`;
        }
      } catch (e) {
        if (genStatus) {
          genStatus.textContent = `Generate failed: ${(e as Error).message}`;
        }
      } finally {
        genBtn.disabled = false;
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
    <nav class="curator-tabbar" aria-label="Mode">
      <button class="on" data-tab="curator">Curator</button>
      <button data-tab="rate" id="curator-tab-rate">Rate</button>
      <button data-tab="calibrate" id="curator-tab-calibrate">Calibrate</button>
    </nav>
    <button class="curator-exit" id="exit" aria-label="Exit curator">×</button>
  </header>`;
}

function bindNav(root: HTMLElement, onExit: () => void): void {
  root.querySelector<HTMLButtonElement>('#exit')?.addEventListener('click', onExit);
  // Curator-internal tab bar: Rate/Calibrate just exit back to the main shell
  // (which has its own tab handlers). Keeps the visual contract from §2.1
  // without each subscreen owning a parallel router.
  root.querySelectorAll<HTMLButtonElement>('.curator-tabbar button').forEach((b) => {
    if (b.classList.contains('on')) return;
    b.addEventListener('click', () => onExit());
  });
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

function showPeekOverlay(
  host: HTMLElement,
  c: Candidate | null,
  gate: BppGate | null,
  license: LicensePolicy | null,
): void {
  if (!c) return;
  hidePeekOverlay(host);
  const overlay = document.createElement('div');
  overlay.className = 'curator-peek-overlay';
  const dims = c.width && c.height ? `${c.width}×${c.height}` : 'unknown dims';
  const sz = c.size_bytes ? `${(c.size_bytes / 1024).toFixed(0)} KB` : '?';
  const fmt = c.format ?? '?';
  const cat = c.suspected_category ? `<div class="peek-row"><span>category</span><span>${escapeHtml(c.suspected_category)}</span></div>` : '';
  const bppLine = gate?.bpp != null ? `<div class="peek-row"><span>bpp</span><span>${gate.bpp.toFixed(2)} (${gate.verdict})</span></div>` : '';
  const qLine = c.source_q_detected != null
    ? `<div class="peek-row"><span>source q</span><span>${c.source_q_detected.toFixed(1)}</span></div>`
    : '';
  const licLine = license
    ? `<div class="peek-row"><span>license</span><span>${escapeHtml(license.label)}</span></div>`
    : '';
  overlay.innerHTML = `
    <div class="peek-card">
      <div class="peek-row peek-sha"><span>sha256</span><code>${escapeHtml(c.sha256.slice(0, 16))}…</code></div>
      <div class="peek-row"><span>corpus</span><span>${escapeHtml(c.corpus)}</span></div>
      ${cat}
      <div class="peek-row"><span>format</span><span>${escapeHtml(fmt)}</span></div>
      <div class="peek-row"><span>dims</span><span>${dims}</span></div>
      <div class="peek-row"><span>size</span><span>${sz}</span></div>
      ${bppLine}
      ${qLine}
      ${licLine}
    </div>
  `;
  host.appendChild(overlay);
}

function hidePeekOverlay(host: HTMLElement): void {
  host.querySelectorAll('.curator-peek-overlay').forEach((el) => el.remove());
}

function renderBppGate(gate: BppGate | null): string {
  if (!gate) return '';
  const cls =
    gate.verdict === 'Low' ? 'bpp-low'
    : gate.verdict === 'High' ? 'bpp-high'
    : gate.verdict === 'Ok' ? 'bpp-ok'
    : 'bpp-unknown';
  const icon =
    gate.verdict === 'Low' ? '⚠'
    : gate.verdict === 'High' ? '✓'
    : gate.verdict === 'Ok' ? '✓'
    : 'ℹ';
  return `<div class="curator-bpp-gate ${cls}" data-verdict="${gate.verdict}" role="status" aria-live="polite">
    <span class="curator-bpp-icon">${icon}</span>
    <span class="curator-bpp-msg">${escapeHtml(gate.message)}</span>
  </div>`;
}

interface CropOffset {
  x: number;
  y: number;
}

function paintSplit(
  left: HTMLCanvasElement,
  right: HTMLCanvasElement,
  img: HTMLImageElement,
  offset: CropOffset = { x: 0, y: 0 },
): void {
  const dpr = window.devicePixelRatio || 1;
  const naturalW = img.naturalWidth;
  const naturalH = img.naturalHeight;

  // Container queries: pick a window centered on the image, then translate
  // by the offset (in image pixels) clamped to the source extent.
  const leftRect = left.getBoundingClientRect();
  const rightRect = right.getBoundingClientRect();

  // Left: 1:1 device pixels — canvas backing = (cssW * dpr), draw image 1:1 device px
  const lcssW = Math.max(1, Math.floor(leftRect.width));
  const lcssH = Math.max(1, Math.floor(leftRect.height));
  left.width = lcssW * dpr;
  left.height = lcssH * dpr;
  const lctx = left.getContext('2d')!;
  lctx.imageSmoothingEnabled = false;
  const lWindowW = Math.min(naturalW, left.width);
  const lWindowH = Math.min(naturalH, left.height);
  const cx0 = Math.floor((naturalW - lWindowW) / 2);
  const cy0 = Math.floor((naturalH - lWindowH) / 2);
  const sxMax = Math.max(0, naturalW - lWindowW);
  const syMax = Math.max(0, naturalH - lWindowH);
  const sx = Math.min(sxMax, Math.max(0, cx0 + offset.x));
  const sy = Math.min(syMax, Math.max(0, cy0 + offset.y));
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

interface SwipeHandlers {
  onRight: () => void;
  onLeft: () => void;
  onDown?: () => void;
  /// Fires after a long-press threshold (320 ms). Use for the spec §2.1
  /// "tap and hold → metadata overlay" gesture.
  onPeekStart?: () => void;
  /// Fires on pointerup or cancel after a peek started.
  onPeekEnd?: () => void;
}

function installSwipe(host: HTMLElement, h: SwipeHandlers): void {
  let startX = 0;
  let startY = 0;
  let down = false;
  let holdTimer: number | null = null;
  let isHolding = false;
  const PEEK_DELAY_MS = 320;

  host.addEventListener('pointerdown', (e: PointerEvent) => {
    down = true;
    startX = e.clientX;
    startY = e.clientY;
    if (h.onPeekStart) {
      holdTimer = window.setTimeout(() => {
        isHolding = true;
        h.onPeekStart?.();
      }, PEEK_DELAY_MS);
    }
  });
  const cancelHold = () => {
    if (holdTimer != null) {
      window.clearTimeout(holdTimer);
      holdTimer = null;
    }
    if (isHolding) {
      h.onPeekEnd?.();
    }
    isHolding = false;
  };
  host.addEventListener('pointerup', (e: PointerEvent) => {
    cancelHold();
    if (!down) return;
    down = false;
    const dx = e.clientX - startX;
    const dy = e.clientY - startY;
    if (Math.abs(dx) > 80 && Math.abs(dx) > Math.abs(dy)) {
      if (dx > 0) h.onRight();
      else h.onLeft();
    } else if (h.onDown && dy > 80 && dy > Math.abs(dx)) {
      h.onDown();
    }
  });
  host.addEventListener('pointercancel', () => {
    cancelHold();
    down = false;
  });
  // If pointer leaves while still down, don't trigger a swipe — but cancel
  // the hold timer so we don't fire onPeekStart on an already-released touch.
  host.addEventListener('pointerleave', () => {
    if (holdTimer != null) {
      window.clearTimeout(holdTimer);
      holdTimer = null;
    }
  });
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
