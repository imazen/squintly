// Which variant is on screen, as one function of "what is currently held".
//
// This used to be four places that each half-decided the answer: `viewForPress`
// (pointer), `viewFromHeld` (release), a keyboard `cycle()` that *toggled*
// instead of holding, and a space-bar branch. They disagreed — the right mouse
// button meant B in one mode and nothing in another, and the arrow keys stepped
// through a carousel while the mouse held a view down. Centralising it is the
// point of this module: one table, one stack, no per-call-site reasoning.

export type View = 'a' | 'b' | 'ref';
export type InputMode = 'tap' | 'hold' | 'buttons';

/// What a press was made with. Keyboard and pointer are the same kind of thing
/// here — both are held, both stack.
export type Button =
  | 'lmb'
  | 'rmb'
  | 'touch'
  | 'arrow-left'
  | 'arrow-right'
  | 'space';

export interface PressContext {
  mode: InputMode;
  /// Pair trials have A and B; single-stimulus trials have only the encoding
  /// and the reference, so "show B" has to collapse to something real.
  isPair: boolean;
  /// Which half of the frame a pointer press landed in. `null` for keyboard,
  /// which has no position.
  half: 'left' | 'right' | null;
}

/// What is on screen when nothing is held.
///
/// `tap` rests on the encoding being judged and peeks at the reference; `hold`
/// and `buttons` invert that — the reference is the resting view and a press
/// flicks to a variant.
export function restingView(mode: InputMode): View {
  return mode === 'tap' ? 'a' : 'ref';
}

/**
 * THE LOGIC TABLE — what a press selects.
 *
 * | input        | tap        | hold           | buttons |
 * |--------------|------------|----------------|---------|
 * | LMB          | ref (peek) | half: L→a R→b  | a       |
 * | RMB          | **b**      | **b**          | **b**   |
 * | touch        | ref (peek) | half: L→a R→b  | a       |
 * | ArrowLeft    | a          | a              | a       |
 * | ArrowRight   | **b**      | **b**          | **b**   |
 * | Space        | ref        | ref            | ref     |
 *
 * Two rules do the work:
 *
 *  * **The right button always means B**, in every mode. It is the one binding
 *    that never changes meaning, so "show me the other one" is always available
 *    without knowing which mode you are in.
 *  * **The left button is the mode-dependent one.** Under `hold` it is
 *    positional (which half of the picture you press); under `buttons` it is
 *    plainly A; under `tap` it stays the established press-and-hold peek at the
 *    reference, because that is the gesture `tap` is built around.
 *
 * The arrow keys mirror the buttons — left is A, right is B — rather than
 * mirroring LMB exactly. A keyboard has no pointer position, so the positional
 * reading of LMB has no keyboard analogue, and mapping ArrowLeft to `tap`'s
 * reference-peek would make the *left* arrow show something that is neither
 * left nor a variant. Space is the reference peek on the keyboard.
 *
 * On a single-stimulus trial there is no B, so anything selecting **B**
 * collapses to the other available view: whatever is *not* resting. That keeps
 * those bindings doing something useful rather than going dead. Bindings that
 * select A or the reference are unaffected — both exist on every trial.
 */
export function viewForPress(button: Button, ctx: PressContext): View {
  const { mode, isPair, half } = ctx;
  const other: View = restingView(mode) === 'a' ? 'ref' : 'a';
  const b: View = isPair ? 'b' : other;

  switch (button) {
    case 'rmb':
    case 'arrow-right':
      return b;
    case 'arrow-left':
      // A exists on every trial — pair or single — so this never collapses.
      // Running it through the B-collapse rule made ArrowLeft show the
      // reference on a single-stimulus trial in `tap` mode, which is neither
      // "left" nor a variant.
      return 'a';
    case 'space':
      return 'ref';
    case 'lmb':
    case 'touch':
      if (mode === 'tap') return 'ref';
      if (mode === 'buttons') return 'a';
      // `hold`: the half you press picks the side, matching the view switch and
      // the answer buttons. Decided on press and held for the whole gesture —
      // re-deciding on move would fight panning, letting a drag across the
      // midline swap the variant out from under a comparison.
      return half === 'right' ? b : 'a';
  }
}

interface Held {
  id: string;
  view: View;
  seq: number;
}

/**
 * The holds currently down, most recent last.
 *
 * The **most recently pressed still-held input wins**, and releasing it falls
 * back to the next one still down rather than to the resting view. That is the
 * behaviour the ordering requirement describes:
 *
 * ```
 * LMB down            → [a]      shows A
 * RMB down            → [a, b]   shows B      (newest wins)
 * RMB up              → [a]      shows A      (falls back, LMB still down)
 *
 * RMB down            → [b]      shows B
 * LMB down            → [b, a]   shows A
 * LMB up              → [b]      shows B      (falls back to RMB)
 *
 * RMB down, LMB down  → [b, a]   shows A
 * RMB up              → [a]      shows A      (unchanged — A was on top)
 * RMB down            → [a, b]   shows B
 * ```
 *
 * A plain "current button wins" rule gets the first line right and the rest
 * wrong; a plain "first press wins" gets the opposite set wrong. Only a stack
 * gives all of them.
 */
export class HoldStack {
  private held: Held[] = [];
  private seq = 0;

  /// Idempotent per id: a repeated press (auto-repeat, a re-entrant pointer
  /// event) refreshes the existing entry instead of stacking duplicates that
  /// would each need their own release.
  press(id: string, view: View): void {
    this.release(id);
    this.held.push({ id, view, seq: this.seq++ });
  }

  release(id: string): void {
    this.held = this.held.filter((h) => h.id !== id);
  }

  clear(): void {
    this.held = [];
  }

  get size(): number {
    return this.held.length;
  }

  has(id: string): boolean {
    return this.held.some((h) => h.id === id);
  }

  /// The view the current holds imply, or `null` when nothing is held — the
  /// caller substitutes the resting view.
  get current(): View | null {
    if (this.held.length === 0) return null;
    return this.held.reduce((a, b) => (b.seq > a.seq ? b : a)).view;
  }

  /// What should be on screen right now, holds or not.
  ///
  /// `resting` is passed in rather than derived from the mode alone because
  /// under `tap` the resting view is whichever variant the observer last
  /// selected with the view switch — releasing a peek at the reference must
  /// return to *that*, not always to A.
  resolve(resting: View): View {
    return this.current ?? resting;
  }
}

/// Which of our buttons a `PointerEvent.buttons` bit corresponds to.
///
/// The bitmask and the `button` field disagree on ordering, which is a standing
/// trap: `button` is 0=left, 1=middle, 2=right, while `buttons` is 1=left,
/// 2=right, 4=middle.
const BUTTON_BITS: ReadonlyArray<{ bit: number; button: number; name: Button }> = [
  { bit: 1, button: 0, name: 'lmb' },
  { bit: 2, button: 2, name: 'rmb' },
];

export interface ButtonDelta {
  pressed: Array<{ id: string; button: Button }>;
  released: string[];
}

/**
 * Reconcile held mouse buttons from a `buttons` bitmask.
 *
 * **A second button press fires no `pointerdown`.** Per Pointer Events,
 * `pointerdown` fires only when the pointer transitions from *no* buttons to
 * some button; pressing another while one is held arrives as a `pointermove`
 * with an updated `buttons` mask, and releasing one of two likewise fires no
 * `pointerup`. Measured in Chromium: LMB down → `pointerdown buttons=1`, then
 * RMB down → only `contextmenu buttons=3`, then both up → a single
 * `pointerup buttons=0`.
 *
 * So button state cannot be driven off down/up events at all. Diffing the mask
 * on every pointer event is the only way to see the middle of that sequence.
 */
export function diffButtons(prev: number, next: number, pointerId: number): ButtonDelta {
  const out: ButtonDelta = { pressed: [], released: [] };
  for (const { bit, button, name } of BUTTON_BITS) {
    const was = (prev & bit) !== 0;
    const now = (next & bit) !== 0;
    const id = holdIdFor('mouse', pointerId, button);
    if (!was && now) out.pressed.push({ id, button: name });
    else if (was && !now) out.released.push(id);
  }
  return out;
}

/// Identity of a held pointer input.
///
/// A mouse reports **every button on the same `pointerId`**, so keying holds by
/// pointer id makes the second button replace the first and one release drop
/// both. Mouse holds are therefore keyed by *button*; touches by pointer, since
/// each finger is genuinely a separate contact.
export function holdIdFor(pointerType: string, pointerId: number, button: number): string {
  return pointerType === 'mouse' ? `m${button}` : `p${pointerId}`;
}

/// Map a `PointerEvent.button` to our vocabulary. Touch and pen report button 0
/// with `pointerType` set, and a touch has no second button to press.
export function buttonFor(pointerType: string, button: number): Button | null {
  if (pointerType !== 'mouse') return 'touch';
  if (button === 0) return 'lmb';
  if (button === 2) return 'rmb';
  return null; // middle / back / forward: not bound
}

/// Map a keyboard event key to our vocabulary.
export function buttonForKey(key: string): Button | null {
  if (key === 'ArrowLeft') return 'arrow-left';
  if (key === 'ArrowRight') return 'arrow-right';
  if (key === ' ') return 'space';
  return null;
}
