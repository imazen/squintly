// Li 2020 virtual chinrest, simplified. Two stages:
//   1) Card resize → CSS px per mm.
//   2) Blind-spot sweep → viewing distance.
// Both are skippable; we record null. Stage 1 alone gives huge value (you can always
// fall back to a default 30 cm viewing distance for phones if the blind-spot is too
// hard for crowdsourced subjects).

const CARD_MM_W = 85.6;
const CARD_MM_H = 53.98;

export function renderCalibration(
  root: HTMLElement,
  onDone: (result: { css_px_per_mm: number | null; viewing_distance_cm: number | null }) => void,
): void {
  // Stage 1: card resize
  let pxPerMm: number | null = null;
  root.innerHTML = `
    <div class="screen center">
      <h1>Calibration: hold a card to your screen</h1>
      <p class="muted">Find any card the size of a debit/credit/transit card. Drag the slider until the on-screen rectangle matches its size.</p>
      <div id="card" class="card-mock">credit-card sized</div>
      <input id="slider" type="range" min="80" max="600" step="1" value="200" />
      <div class="choice-row" style="max-width: 360px; width: 100%;">
        <button id="skip">Skip</button>
        <button id="next" class="primary">Looks right</button>
      </div>
    </div>
  `;
  const card = root.querySelector<HTMLDivElement>('#card')!;
  const slider = root.querySelector<HTMLInputElement>('#slider')!;
  const updateCard = () => {
    const widthPx = parseInt(slider.value, 10);
    pxPerMm = widthPx / CARD_MM_W;
    card.style.width = `${widthPx}px`;
    card.style.height = `${pxPerMm * CARD_MM_H}px`;
  };
  slider.addEventListener('input', updateCard);
  updateCard();
  root.querySelector<HTMLButtonElement>('#skip')!.addEventListener('click', () => onDone({ css_px_per_mm: null, viewing_distance_cm: null }));
  root.querySelector<HTMLButtonElement>('#next')!.addEventListener('click', () => stage2(root, pxPerMm, onDone));
}

function stage2(
  root: HTMLElement,
  pxPerMm: number | null,
  onDone: (r: { css_px_per_mm: number | null; viewing_distance_cm: number | null }) => void,
): void {
  // Stage 2: blind-spot sweep. The blind spot is ~13.5° from the fovea on the
  // horizontal meridian. We fixate a left dot, sweep a right dot inward, user taps
  // when it disappears. Distance = horizontal_distance_mm / tan(13.5°).
  if (!pxPerMm) {
    // Without mm calibration we can't compute a distance from the sweep; ask the
    // user to pick a preset bucket instead.
    root.innerHTML = `
      <div class="screen center">
        <h1>Roughly how close is your screen?</h1>
        <p class="muted">Pick the closest match.</p>
        <div class="choice-row" style="max-width: 360px; width: 100%; flex-direction: column;">
          <button data-d="25">Very close (~25 cm)</button>
          <button data-d="35">Phone in hand (~35 cm)</button>
          <button data-d="50">Lap (~50 cm)</button>
          <button data-d="70">Desk (~70 cm)</button>
          <button data-d="150">Across the room (~150 cm)</button>
          <button data-d="0">Skip</button>
        </div>
      </div>
    `;
    root.querySelectorAll<HTMLButtonElement>('button[data-d]').forEach((b) => {
      b.addEventListener('click', () => {
        const d = parseInt(b.dataset.d || '0', 10);
        onDone({ css_px_per_mm: pxPerMm, viewing_distance_cm: d > 0 ? d : null });
      });
    });
    return;
  }
  // Real blind-spot UI
  let raf = 0;
  let dotX = 0;
  let started = false;
  let dist: number | null = null;
  root.innerHTML = `
    <div class="screen center">
      <h1>Blind-spot test</h1>
      <p class="muted">Close your right eye. Stare at the left × . When the red dot disappears, tap the screen.</p>
      <div id="stage" style="position: relative; width: 100%; height: 320px; background: #000; border-radius: 12px; overflow: hidden;">
        <div style="position: absolute; left: 24px; top: 50%; transform: translateY(-50%); color: white; font-size: 32px; line-height: 1;">×</div>
        <div id="dot" style="position: absolute; width: 18px; height: 18px; border-radius: 50%; background: red; top: 50%; transform: translate(-50%, -50%); right: 32px;"></div>
      </div>
      <div class="choice-row" style="max-width: 360px; width: 100%;">
        <button id="start" class="primary">Start sweep</button>
        <button id="skip2">Skip</button>
      </div>
      <p id="result" class="muted"></p>
    </div>
  `;
  const stage = root.querySelector<HTMLDivElement>('#stage')!;
  const dot = root.querySelector<HTMLDivElement>('#dot')!;
  const result = root.querySelector<HTMLParagraphElement>('#result')!;
  const finish = (d: number | null) => {
    cancelAnimationFrame(raf);
    onDone({ css_px_per_mm: pxPerMm, viewing_distance_cm: d });
  };
  root.querySelector<HTMLButtonElement>('#skip2')!.addEventListener('click', () => finish(null));

  root.querySelector<HTMLButtonElement>('#start')!.addEventListener('click', () => {
    if (started) return;
    started = true;
    const stageRect = stage.getBoundingClientRect();
    dotX = stageRect.width - 32;
    const stepPxPerFrame = stageRect.width / (60 * 8); // ~8 second sweep
    const tick = () => {
      dotX -= stepPxPerFrame;
      dot.style.right = `${stageRect.width - dotX}px`;
      if (dotX > 80) raf = requestAnimationFrame(tick);
      else finish(null); // timed out
    };
    raf = requestAnimationFrame(tick);
  });
  stage.addEventListener('click', () => {
    if (!started || dist !== null) return;
    cancelAnimationFrame(raf);
    const stageRect = stage.getBoundingClientRect();
    const xMarker = 24 + 16; // marker left + half width
    const horizCss = (stageRect.width - dotX) - xMarker; // wait — dot is positioned via right
    // Recompute properly: the dot's CSS-x position is `dotX` from the LEFT edge.
    const horizCssCorrect = dotX - xMarker;
    const horizMm = horizCssCorrect / pxPerMm!;
    const distMm = horizMm / Math.tan((13.5 * Math.PI) / 180);
    dist = Math.round(distMm / 10);
    result.textContent = `Estimated distance: ${dist} cm. Tap once more to confirm.`;
    setTimeout(() => finish(dist), 1500);
    void horizCss; // silence unused
  });
}
