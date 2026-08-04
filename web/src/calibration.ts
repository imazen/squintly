import { loadCalibration } from './conditions';

// Li 2020 virtual chinrest, simplified. Two stages:
//   1) Card resize → CSS px per mm.
//   2) Blind-spot sweep → viewing distance.
// Both are skippable; we record null. Stage 1 alone gives huge value (you can always
// fall back to a default 30 cm viewing distance for phones if the blind-spot is too
// hard for crowdsourced subjects).

const CARD_MM_W = 85.6;
/// Eccentricity of the blind spot from fixation, degrees. Reported range is
/// 12-15°; this is the midpoint.
const BLIND_SPOT_DEG = 13.5;
const CARD_MM_H = 53.98;

export function renderCalibration(
  root: HTMLElement,
  onDone: (result: { css_px_per_mm: number | null; viewing_distance_cm: number | null }) => void,
): void {
  // Stage 1: card resize.
  //
  // Seeded from whatever was measured last time. This used to open at a fixed
  // slider value every time, so a returning observer re-did a measurement the
  // app had already stored — and `Skip` returned nulls that the caller saved
  // straight over the good value, silently discarding it.
  const prior = loadCalibration();
  let pxPerMm: number | null = prior.css_px_per_mm;
  root.innerHTML = `
    <div class="screen center">
      <h1>Calibration: hold a card to your screen</h1>
      <p class="muted">Find any card the size of a debit/credit/transit card. Drag the slider until the on-screen rectangle matches its size — turn the card if it does not fit across your screen.</p>
      ${
        prior.css_px_per_mm
          ? `<p class="muted">Already calibrated on this device${
              prior.viewing_distance_cm ? ` at ~${prior.viewing_distance_cm} cm` : ''
            } — adjust only if something changed.</p>`
          : ''
      }
      <div id="card" class="card-mock"><span>credit-card sized</span></div>
      <input id="slider" type="range" min="80" max="700" step="1" value="${Math.round(
        (prior.css_px_per_mm ?? 200 / CARD_MM_W) * CARD_MM_W,
      )}" />
      <button id="rotate-card" class="card-rotate">↻ Turn the card</button>
      <div class="choice-row" style="max-width: 360px; width: 100%;">
        <button id="skip">Skip</button>
        <button id="next" class="primary">Looks right</button>
      </div>
    </div>
  `;
  const card = root.querySelector<HTMLDivElement>('#card')!;
  const slider = root.querySelector<HTMLInputElement>('#slider')!;

  /// Which way the card lies on screen.
  ///
  /// A card is 85.6mm along its long edge and a phone is about 65mm wide, so a
  /// LANDSCAPE card physically cannot fit across a portrait phone — the slider
  /// ran out of travel before the rectangle reached a real card, which made
  /// calibration impossible on the device this study mostly runs on. Turning
  /// the card puts its long edge down the screen, where there is room.
  ///
  /// Measuring along either axis is equivalent: CSS pixels are square, so
  /// mm-per-px is the same horizontally and vertically. The slider therefore
  /// keeps meaning "the long edge, in px" in both orientations, and a value
  /// stored in one is valid in the other.
  let upright = window.innerHeight > window.innerWidth;
  const updateCard = () => {
    const longPx = parseInt(slider.value, 10);
    pxPerMm = longPx / CARD_MM_W;
    const shortPx = pxPerMm * CARD_MM_H;
    card.style.width = `${upright ? shortPx : longPx}px`;
    card.style.height = `${upright ? longPx : shortPx}px`;
    card.classList.toggle('upright', upright);
  };
  slider.addEventListener('input', updateCard);
  root.querySelector<HTMLButtonElement>('#rotate-card')!.addEventListener('click', () => {
    upright = !upright;
    updateCard();
  });
  updateCard();
  // Skip keeps whatever was already measured. Returning nulls here meant the
  // caller wrote nulls over a good calibration — skipping is "leave it alone",
  // not "throw it away".
  root
    .querySelector<HTMLButtonElement>('#skip')!
    .addEventListener('click', () => onDone(prior));
  root.querySelector<HTMLButtonElement>('#next')!.addEventListener('click', () => stage2(root, pxPerMm, onDone));
}

function stage2(
  root: HTMLElement,
  pxPerMm: number | null,
  onDone: (r: { css_px_per_mm: number | null; viewing_distance_cm: number | null }) => void,
): void {
  // Stage 2: blind-spot sweep. Fixate the × on the LEFT, sweep a dot inward from
  // the right, tap when it vanishes. Distance = horizontal_mm / tan(eccentricity).
  //
  // WHICH EYE IS NOT ARBITRARY. The optic disc sits on the *nasal* retina, and
  // the optics invert, so each eye's blind spot lies in its *temporal* (outer)
  // visual field ~12-15° from fixation: the left eye's is to the LEFT, the right
  // eye's to the RIGHT. Here the target is always to the RIGHT of fixation, so
  // the only eye that can lose it is the RIGHT one — the left eye must be the
  // one that is closed.
  //
  // This said "close your right eye", i.e. view with the left, which put the
  // target in that eye's nasal field where there is no blind spot. The dot could
  // never disappear, so the sweep always ran to its timeout and returned no
  // distance. It looked like a working feature that simply never produced a
  // measurement. If the layout is ever mirrored, this instruction has to mirror
  // with it.
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
      <p class="muted">Close your <strong>left</strong> eye. Stare at the × with your right eye and hold still. Tap the moment the red dot vanishes.</p>
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
    // `dotX` is the dot's x from the stage's LEFT edge; the × marker sits at
    // 24px with a glyph roughly 18px wide, so its centre is ~33px.
    const xMarker = 24 + 9;
    const horizMm = (dotX - xMarker) / pxPerMm!;
    // 12-15° is the reported range; 13.5° is the midpoint. The estimate is only
    // as good as that assumption, which is why it is a coarse bucket downstream.
    const distMm = horizMm / Math.tan((BLIND_SPOT_DEG * Math.PI) / 180);
    dist = Math.round(distMm / 10);
    result.textContent = `Estimated distance: ${dist} cm. Tap once more to confirm.`;
    setTimeout(() => finish(dist), 1500);
  });
}
