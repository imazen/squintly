// Onboarding calibration: 5 fixed test trials at session start with answer
// feedback. Implements docs/methodology.md §3.7 (CID22 + KonIQ + Foldit
// pattern). Soft-fails under 60% — observers.calibrated=0 — but still lets
// them sample.

export interface CalibrationItem {
  id: string;
  kind: 'single' | 'pair' | 'imc';
  description: string;
  source_url: string | null;
  a_url: string | null;
  b_url: string | null;
  a_codec: string | null;
  b_codec: string | null;
  a_quality: number | null;
  b_quality: number | null;
  intrinsic_w: number | null;
  intrinsic_h: number | null;
  feedback_text: string | null;
}

export interface CalibrationListResp { items: CalibrationItem[] }
export interface CalibrationAck {
  correct: boolean;
  expected_choice: string;
  feedback_text: string | null;
}
export interface CalibrationFinalize {
  calibrated: boolean;
  score: number;
}

export async function fetchCalibration(): Promise<CalibrationListResp> {
  const r = await fetch('/api/calibration');
  if (!r.ok) throw new Error(`fetchCalibration ${r.status}`);
  return r.json();
}

export async function postCalibrationResponse(body: {
  session_id: string;
  observer_id: string;
  pool_id: string;
  choice: string;
  dwell_ms: number;
}): Promise<CalibrationAck> {
  const r = await fetch('/api/calibration/response', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });
  if (!r.ok) throw new Error(`postCalibrationResponse ${r.status}`);
  return r.json();
}

export async function finalizeCalibration(observer_id: string): Promise<CalibrationFinalize> {
  const r = await fetch('/api/calibration/finalize', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ observer_id }),
  });
  if (!r.ok) throw new Error(`finalizeCalibration ${r.status}`);
  return r.json();
}

/**
 * Renders the calibration sequence. When all items are done (or the pool is
 * empty), calls `onDone` with the finalize result. Always non-blocking — even
 * a low score lets the observer continue to real trials; their data carries
 * `calibrated=0` and is filtered at training time.
 */
export async function runCalibration(
  root: HTMLElement,
  ctx: { session_id: string; observer_id: string },
  onDone: (result: CalibrationFinalize) => void,
): Promise<void> {
  let resp: CalibrationListResp;
  try {
    resp = await fetchCalibration();
  } catch (e) {
    // No calibration pool seeded — skip silently.
    onDone({ calibrated: true, score: 1.0 });
    return;
  }
  if (resp.items.length === 0) {
    onDone({ calibrated: true, score: 1.0 });
    return;
  }
  let i = 0;
  const renderItem = () => {
    if (i >= resp.items.length) {
      finalizeCalibration(ctx.observer_id).then(onDone).catch(() => onDone({ calibrated: true, score: 1.0 }));
      return;
    }
    const item = resp.items[i];
    const shownAt = performance.now();

    if (item.kind === 'imc') {
      // Instructed-response check (Meade & Craig 2012). Pure text task.
      root.innerHTML = `
        <div class="screen center">
          <h1>Quick check</h1>
          <p>${escapeHtml(item.description)}</p>
          <div class="choice-row" style="max-width: 360px;">
            <button data-c="1">1</button>
            <button data-c="2">2</button>
            <button data-c="tie">tie</button>
            <button data-c="4">4</button>
          </div>
          <p class="muted">Item ${i + 1} of ${resp.items.length}</p>
        </div>
      `;
    } else {
      const stim = item.kind === 'pair' ? item.a_url : item.a_url;
      const buttons = item.kind === 'single'
        ? `
          <button data-c="1">1 imperceptible</button>
          <button data-c="2">2 I notice</button>
          <button data-c="3">3 I dislike</button>
          <button data-c="4">4 I hate</button>`
        : `
          <button data-c="a">A closer</button>
          <button data-c="tie">can't tell</button>
          <button data-c="b">B closer</button>`;
      root.innerHTML = `
        <div class="screen">
          <p class="muted" style="text-align:center;">Calibration ${i + 1} of ${resp.items.length}</p>
          <div style="max-width: 480px; margin: 0 auto;">
            <p>${escapeHtml(item.description)}</p>
            ${stim ? `<img src="${stim}" alt="" style="width:100%;border-radius:8px;" />` : ''}
            ${item.kind === 'pair' && item.b_url ? `<img src="${item.b_url}" alt="" style="width:100%;border-radius:8px;margin-top:8px;" />` : ''}
            <div class="choice-row" style="margin-top:16px;flex-wrap:wrap;">${buttons}</div>
          </div>
        </div>
      `;
    }
    root.querySelectorAll<HTMLButtonElement>('button[data-c]').forEach((b) => {
      b.addEventListener('click', async () => {
        const choice = b.dataset.c!;
        const dwell_ms = Math.round(performance.now() - shownAt);
        try {
          const ack = await postCalibrationResponse({
            session_id: ctx.session_id,
            observer_id: ctx.observer_id,
            pool_id: item.id,
            choice,
            dwell_ms,
          });
          showFeedback(root, ack.correct, ack.feedback_text || (ack.correct ? 'Correct' : `Expected: ${ack.expected_choice}`));
        } catch (e) {
          showFeedback(root, false, 'Could not save your answer; continuing.');
        }
        setTimeout(() => { i += 1; renderItem(); }, 1400);
      });
    });
  };
  renderItem();
}

function showFeedback(root: HTMLElement, correct: boolean, text: string) {
  const banner = document.createElement('div');
  banner.style.cssText = `position:fixed;left:50%;bottom:24px;transform:translateX(-50%);
    background:${correct ? '#1f3a26' : '#3a1f25'};border:1px solid ${correct ? 'var(--good)' : 'var(--warn)'};
    color:${correct ? 'var(--good)' : 'var(--warn)'};padding:10px 16px;border-radius:10px;
    font-size:0.95rem;z-index:100;`;
  banner.textContent = text;
  root.appendChild(banner);
  setTimeout(() => banner.remove(), 1300);
}

function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);
}
