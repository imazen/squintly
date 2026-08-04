// Ask once how the observer wants to drive the comparison.
//
// The mode was previously chosen *for* people by device class and buried in a
// dropdown on the trial screen, next to the zoom controls, labelled
// "Interaction mode". So the single decision that most changes what the task
// feels like — whether the reference is what you see at rest, and what your
// hand does to switch — was discoverable only by someone already fluent enough
// in the UI to go looking for it. Everyone else rated a whole session in
// whatever the default happened to be, without knowing the alternatives
// existed.
//
// It is also the one screen where a how-to is actually read: immediately
// before the first trial, about the gesture you are about to use. The same
// text on a help page is not read at all.
//
// Shown once. `hasChosenInputMode` is what makes it once rather than every
// load, and it is deliberately false for the observers already in the study —
// they were never asked, so they get the prompt on their next visit.

import {
  ALL_INPUT_MODES,
  availableInputModes,
  INPUT_MODE_LABELS,
  INPUT_MODE_LABELS_LONG,
  loadInputMode,
  saveInputMode,
  type InputMode,
} from './input-mode';

/// What the hand actually does, per mode. Deliberately concrete — "left button
/// for A" rather than "select a variant" — because this is the text someone
/// reads instead of experimenting.
const HOW_TO: Record<InputMode, string[]> = {
  buttons: [
    'Hold the <b>left mouse button</b> to see A.',
    'Hold the <b>right mouse button</b> to see B.',
    'Let go to return to the <b>original</b>.',
  ],
  hold: [
    'Press and hold the <b>left half</b> of the picture to see A.',
    'Press and hold the <b>right half</b> to see B.',
    'Let go to return to the <b>original</b>.',
  ],
  tap: [
    'Tap <b>A</b>, <b>B</b> or <b>Original</b> under the picture.',
    'Or press and hold the picture to peek at the original.',
    'The right mouse button always shows B.',
  ],
};

/// Why you might want this one. The modes are not ranked; they suit different
/// hands and different hardware.
const WHY: Record<InputMode, string> = {
  buttons:
    'Fastest on a mouse — your eye never leaves the picture and there is nothing to aim at.',
  hold: 'Best on a phone or tablet. Your thumb is already on the glass.',
  tap: 'Steadier if holding a button is awkward, or if you want to study one variant at a time.',
};

const ICON: Record<InputMode, string> = { buttons: '🖱', hold: '👆', tap: '⇄' };

function card(m: InputMode, selected: boolean): string {
  return `
    <button class="mode-card${selected ? ' on' : ''}" data-mode="${m}" role="radio"
            aria-checked="${selected ? 'true' : 'false'}">
      <span class="mode-card-head"><span class="mode-icon" aria-hidden="true">${ICON[m]}</span>
        <span class="mode-card-title">${INPUT_MODE_LABELS[m]}</span></span>
      <span class="mode-card-why">${WHY[m]}</span>
      <ul class="mode-card-how">${HOW_TO[m].map((l) => `<li>${l}</li>`).join('')}</ul>
    </button>`;
}

/**
 * Render the chooser (or, when the device can only drive one mode, a how-to for
 * that mode) and resolve once the observer has moved on.
 *
 * A one-option "choice" is not a choice — it is a screen asking someone to
 * confirm the only thing that could happen. The same content is genuinely
 * useful as instructions, so that case renders as a how-to and says what the
 * gesture is instead of pretending to offer alternatives.
 */
export function chooseInputMode(root: HTMLElement): Promise<InputMode> {
  const modes = availableInputModes();
  // Present in a stable order regardless of what the device filtered out, so
  // the list does not reshuffle between a phone and a desktop.
  const ordered = ALL_INPUT_MODES.filter((m) => modes.includes(m));
  const initial = loadInputMode();
  let picked: InputMode = ordered.includes(initial) ? initial : ordered[0];

  return new Promise((resolve) => {
    if (ordered.length === 1) {
      const only = ordered[0];
      root.innerHTML = `
        <div class="screen center" data-screen="mode-howto">
          <h1>How to compare</h1>
          <p class="muted">${INPUT_MODE_LABELS_LONG[only]}</p>
          <div class="mode-grid one">${card(only, true)}</div>
          <div class="row">
            <button id="mode-continue" class="primary">Got it</button>
          </div>
        </div>`;
      root.querySelector<HTMLButtonElement>('#mode-continue')!.addEventListener('click', () => {
        saveInputMode(only);
        resolve(only);
      });
      return;
    }

    root.innerHTML = `
      <div class="screen center" data-screen="mode-choose">
        <h1>How would you like to compare?</h1>
        <p class="muted">You are shown two versions of the same picture and asked which is
          closer to the original. Pick whichever switching feels natural — you can change it
          any time from the menu.</p>
        <div class="mode-grid" role="radiogroup" aria-label="Comparison mode">
          ${ordered.map((m) => card(m, m === picked)).join('')}
        </div>
        <div class="row">
          <button id="mode-continue" class="primary">Start rating</button>
        </div>
      </div>`;

    const cards = [...root.querySelectorAll<HTMLButtonElement>('.mode-card')];
    for (const el of cards) {
      el.addEventListener('click', () => {
        picked = el.dataset.mode as InputMode;
        for (const x of cards) {
          const on = x === el;
          x.classList.toggle('on', on);
          x.setAttribute('aria-checked', on ? 'true' : 'false');
        }
      });
    }
    root.querySelector<HTMLButtonElement>('#mode-continue')!.addEventListener('click', () => {
      saveInputMode(picked);
      resolve(picked);
    });
  });
}
