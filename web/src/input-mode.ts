// How the observer drives the trial UI.
//
// This is a genuine choice, not a preference toggle: the two modes put the
// reference in different places, so they suit different comparisons.
//
//  * `tap`  — the encoding is on screen; tap A / B / Original, or press and
//    hold the image to peek at the reference.
//  * `buttons` — same resting state as `hold`, but the *button* picks the side:
//    left for A, right for B. Wanted by mouse users who would rather not move
//    the pointer across the picture to switch. Desktop only, and it needs the
//    context menu suppressed; `hold` exists because that combination cannot
//    work on a phone.
//  * `hold` — the *reference* is what you see at rest; press and hold the
//    **left half** of the picture to flick to A, the **right half** for B, and
//    release to snap back. Much faster for spotting a difference, because the
//    eye stays fixed and the picture changes under it rather than the other way
//    round.
//
// `hold` splits by *where you press*, not by which mouse button. Buttons were
// the first attempt and were wrong twice over: a touchscreen has no second
// button, so the mode was desktop-only for no good reason on a phone-first
// study; and it needed the context menu suppressed. Halves work with a thumb,
// need no suppression, and land where the labels already are — A on the left
// and B on the right, matching the view switch and the answer buttons.
//
// The mode is recorded on every response (`input_mode`), because it changes
// what `reveal_ms_total` measures — under `hold` the reference is the resting
// state, so that column is naturally large. See migration 0017.

export type InputMode = 'tap' | 'hold' | 'buttons';

export const ALL_INPUT_MODES: InputMode[] = ['tap', 'hold', 'buttons'];

/// Modes this device can actually drive.
export function availableInputModes(): InputMode[] {
  return ALL_INPUT_MODES.filter((m) => m !== 'buttons' || supportsButtonsMode());
}

const KEY = 'squintly_input_mode';

export function isInputMode(v: string | null): v is InputMode {
  return v === 'tap' || v === 'hold' || v === 'buttons';
}

/// `buttons` needs distinct left/right mouse buttons, which a touchscreen has
/// no equivalent for. `hold` covers the same idea with a thumb, so this is an
/// alternative for mouse users rather than the only way in.
export function supportsButtonsMode(): boolean {
  return (
    typeof window !== 'undefined' &&
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(pointer: fine)').matches
  );
}

/**
 * Every pointer can press one half of a picture, so `hold` is offered
 * everywhere. It was gated to `pointer: fine` while it depended on left/right
 * mouse buttons — a restriction that came from the binding, not the idea.
 */
export function supportsHoldMode(): boolean {
  return true;
}

/// Touch-primary devices default to `hold`.
///
/// On a phone the segmented control is three small targets below the picture,
/// and every switch is a look away from the thing being compared. Holding one
/// half of the image keeps the eye on the stimulus and changes the picture
/// under it, which is the comparison you actually want to make — and a thumb
/// is already on the glass. On a mouse, `tap` stays the default: the pointer
/// costs nothing to move and a click is not a sustained gesture.
///
/// `pointer: coarse` rather than a touch-capability check: a laptop with a
/// touchscreen is still mouse-primary, and its owner should get the mouse
/// default.
function defaultInputMode(): InputMode {
  if (
    typeof window !== 'undefined' &&
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(pointer: coarse)').matches
  ) {
    return 'hold';
  }
  return 'tap';
}

export function loadInputMode(): InputMode {
  try {
    const v = localStorage.getItem(KEY);
    // An explicit choice always wins over the device default — that is what
    // makes the setting stick. A stored mode this device cannot drive would
    // strand the observer with an inoperable UI, so fall back rather than
    // honour that one.
    if (isInputMode(v) && (v !== 'buttons' || supportsButtonsMode())) return v;
  } catch {
    /* private mode / storage disabled */
  }
  return defaultInputMode();
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
  hold: 'Hold a half',
  buttons: 'Mouse buttons',
};

export const INPUT_MODE_LABELS_LONG: Record<InputMode, string> = {
  tap: 'Tap to switch between A, B and the original',
  hold: 'Hold one half of the picture; release for the original',
  buttons: 'Left button for A, right for B; release for the original',
};

/**
 * What to tell the observer, given the trial type. `hold` reads differently on
 * a single-stimulus trial, where there is no B to put on the right.
 */
export function inputModeHint(mode: InputMode, isPair: boolean): string {
  if (mode === 'buttons') {
    return isPair
      ? 'hold the left mouse button for A, the right for B · release for the original'
      : 'hold any mouse button to see the compressed version · release for the original';
  }
  if (mode === 'hold') {
    return isPair
      ? 'hold the left half for A, the right half for B · release for the original'
      : 'hold the image to see the compressed version · release for the original';
  }
  return isPair
    ? 'tap A / B / Original, or hold the image to peek at the original'
    : 'tap Original, or hold the image to compare';
}
