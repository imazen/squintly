// How the observer drives the trial UI.
//
// This is a genuine choice, not a preference toggle: the two modes put the
// reference in different places, so they suit different comparisons.
//
//  * `tap`  — the encoding is on screen; tap A / B / Original, or press and
//    hold the image to peek at the reference. Works on touch, and is the only
//    mode that makes sense there (a touchscreen has no second button).
//  * `hold` — the *reference* is what you see at rest; hold the left mouse
//    button to flick to A, the right button for B, release to snap back. Much
//    faster for spotting a difference, because the eye stays fixed and the
//    picture changes under it rather than the other way round.
//
// The mode is recorded on every response (`input_mode`), because it changes
// what `reveal_ms_total` measures — under `hold` the reference is the resting
// state, so that column is naturally large. See migration 0017.

export type InputMode = 'tap' | 'hold';

export const INPUT_MODES: InputMode[] = ['tap', 'hold'];

const KEY = 'squintly_input_mode';

export function isInputMode(v: string | null): v is InputMode {
  return v === 'tap' || v === 'hold';
}

/**
 * `hold` needs a mouse: it is driven by distinct left/right buttons and a
 * hold gesture, none of which a touchscreen has. Offering it on a phone would
 * be offering a mode that cannot be operated.
 */
export function supportsHoldMode(): boolean {
  return (
    typeof window !== 'undefined' &&
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(pointer: fine)').matches
  );
}

export function loadInputMode(): InputMode {
  try {
    const v = localStorage.getItem(KEY);
    // A stored `hold` on a device that cannot drive it must not strand the
    // observer with an inoperable UI — fall back rather than honour it.
    if (isInputMode(v) && (v === 'tap' || supportsHoldMode())) return v;
  } catch {
    /* private mode / storage disabled */
  }
  return 'tap';
}

export function saveInputMode(mode: InputMode): void {
  try {
    localStorage.setItem(KEY, mode);
  } catch {
    /* non-fatal: the mode just won't persist */
  }
}

export const INPUT_MODE_LABELS: Record<InputMode, string> = {
  tap: 'Tap to switch',
  hold: 'Hold to compare',
};

export const INPUT_MODE_HINTS: Record<InputMode, string> = {
  tap: 'Tap A / B / Original, or hold the image to peek at the original.',
  hold: 'Hold left mouse for A, right mouse for B. Release to see the original.',
};
