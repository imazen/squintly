import { expect, test } from './fixtures';

import {
  type Button,
  HoldStack,
  type InputMode,
  buttonFor,
  buttonForKey,
  diffButtons,
  holdIdFor,
  restingView,
  viewForPress,
} from '../src/hold-stack';

/// Pure logic — no browser needed, so run it once rather than per device.
test.describe.configure({ mode: 'default' });

const PAIR = { isPair: true, half: null } as const;

test.describe('the logic table', () => {
  // The one binding that must never change meaning: whatever mode you are in,
  // and wherever the pointer is, the right button shows B.
  test('the right button always means B', () => {
    for (const mode of ['tap', 'hold', 'buttons'] as InputMode[]) {
      for (const half of ['left', 'right', null] as const) {
        expect(viewForPress('rmb', { mode, isPair: true, half }), `rmb in ${mode}`).toBe('b');
        expect(
          viewForPress('arrow-right', { mode, isPair: true, half }),
          `ArrowRight in ${mode}`,
        ).toBe('b');
      }
    }
  });

  test('the left button is the mode-dependent one', () => {
    // `tap` rests on the encoding, so the left button peeks at the reference.
    expect(viewForPress('lmb', { mode: 'tap', ...PAIR })).toBe('ref');
    // `buttons` is plainly A.
    expect(viewForPress('lmb', { mode: 'buttons', ...PAIR })).toBe('a');
    // `hold` is positional.
    expect(viewForPress('lmb', { mode: 'hold', isPair: true, half: 'left' })).toBe('a');
    expect(viewForPress('lmb', { mode: 'hold', isPair: true, half: 'right' })).toBe('b');
  });

  test('the arrow keys are always a and b', () => {
    for (const mode of ['tap', 'hold', 'buttons'] as InputMode[]) {
      expect(viewForPress('arrow-left', { mode, ...PAIR }), `ArrowLeft in ${mode}`).toBe('a');
      expect(viewForPress('arrow-right', { mode, ...PAIR }), `ArrowRight in ${mode}`).toBe('b');
    }
  });

  test('space always peeks at the reference', () => {
    for (const mode of ['tap', 'hold', 'buttons'] as InputMode[]) {
      expect(viewForPress('space', { mode, ...PAIR })).toBe('ref');
    }
  });

  test('resting view inverts between tap and the hold modes', () => {
    expect(restingView('tap')).toBe('a');
    expect(restingView('hold')).toBe('ref');
    expect(restingView('buttons')).toBe('ref');
  });

  // A single-stimulus trial has no B. Every binding must still do something
  // rather than going dead, so anything selecting B collapses to the other
  // available view.
  test('on a single-stimulus trial nothing resolves to a nonexistent B', () => {
    const buttons: Button[] = ['lmb', 'rmb', 'touch', 'arrow-left', 'arrow-right', 'space'];
    for (const mode of ['tap', 'hold', 'buttons'] as InputMode[]) {
      for (const b of buttons) {
        for (const half of ['left', 'right'] as const) {
          const v = viewForPress(b, { mode, isPair: false, half });
          expect(v, `${b} in ${mode} on a single trial`).not.toBe('b');
        }
      }
    }
    // A and the reference exist on every trial, so those bindings are
    // unaffected by the collapse — ArrowLeft is A even on a single trial.
    for (const mode of ['tap', 'hold', 'buttons'] as InputMode[]) {
      expect(viewForPress('arrow-left', { mode, isPair: false, half: null })).toBe('a');
      expect(viewForPress('space', { mode, isPair: false, half: null })).toBe('ref');
    }
    // And B collapses to something *other* than what is already resting, so
    // the press visibly does something.
    expect(viewForPress('rmb', { mode: 'tap', isPair: false, half: null })).toBe('ref');
    expect(viewForPress('rmb', { mode: 'hold', isPair: false, half: null })).toBe('a');
    expect(viewForPress('rmb', { mode: 'buttons', isPair: false, half: null })).toBe('a');
  });

  test('pointer and key events map onto the vocabulary', () => {
    expect(buttonFor('mouse', 0)).toBe('lmb');
    expect(buttonFor('mouse', 2)).toBe('rmb');
    expect(buttonFor('mouse', 1), 'middle button is not bound').toBeNull();
    // A touch has no second button, so it is its own input regardless of the
    // `button` field the event carries.
    expect(buttonFor('touch', 0)).toBe('touch');
    expect(buttonFor('pen', 0)).toBe('touch');
    expect(buttonForKey('ArrowLeft')).toBe('arrow-left');
    expect(buttonForKey('ArrowRight')).toBe('arrow-right');
    expect(buttonForKey(' ')).toBe('space');
    expect(buttonForKey('x')).toBeNull();
  });
});

test.describe('hold ordering', () => {
  // Every case from the specification, verbatim. A "current button wins" rule
  // gets the first right and the rest wrong; "first press wins" gets the
  // opposite set wrong. Only a stack satisfies all of them.
  test('LMB held, RMB pressed then released, falls back to A', () => {
    const s = new HoldStack();
    s.press('lmb', 'a');
    expect(s.resolve('ref')).toBe('a');
    s.press('rmb', 'b');
    expect(s.resolve('ref'), 'newest hold wins').toBe('b');
    s.release('rmb');
    expect(s.resolve('ref'), 'LMB is still down, so back to A').toBe('a');
    s.release('lmb');
    expect(s.resolve('ref'), 'nothing held → resting').toBe('ref');
  });

  test('RMB held, LMB pressed then released, falls back to B', () => {
    const s = new HoldStack();
    s.press('rmb', 'b');
    expect(s.resolve('ref')).toBe('b');
    s.press('lmb', 'a');
    expect(s.resolve('ref')).toBe('a');
    s.release('lmb');
    expect(s.resolve('ref'), 'RMB is still down, so back to B').toBe('b');
  });

  test('RMB released while LMB held leaves A, and re-pressing RMB shows B', () => {
    const s = new HoldStack();
    s.press('rmb', 'b');
    s.press('lmb', 'a');
    expect(s.resolve('ref')).toBe('a');
    s.release('rmb');
    expect(s.resolve('ref'), 'A was on top; releasing underneath changes nothing').toBe('a');
    s.press('rmb', 'b');
    expect(s.resolve('ref'), 'pressing RMB again puts B back on top').toBe('b');
  });

  test('a repeated press does not stack duplicates', () => {
    const s = new HoldStack();
    s.press('lmb', 'a');
    s.press('lmb', 'a'); // key auto-repeat, re-entrant pointer event
    s.press('lmb', 'a');
    expect(s.size).toBe(1);
    s.release('lmb');
    expect(s.size, 'one release must clear one hold').toBe(0);
    expect(s.resolve('ref')).toBe('ref');
  });

  test('releasing an unheld input is harmless', () => {
    const s = new HoldStack();
    s.release('rmb');
    expect(s.size).toBe(0);
    s.press('lmb', 'a');
    s.release('rmb');
    expect(s.resolve('ref')).toBe('a');
  });

  test('keyboard and mouse share one stack', () => {
    const s = new HoldStack();
    s.press('karrow-right', 'b');
    expect(s.resolve('ref')).toBe('b');
    // A mouse press on top of a held key wins, and releasing it falls back to
    // the key — the two input kinds are not separate worlds.
    s.press('plmb', 'a');
    expect(s.resolve('ref')).toBe('a');
    s.release('plmb');
    expect(s.resolve('ref')).toBe('b');
    s.release('karrow-right');
    expect(s.resolve('ref')).toBe('ref');
  });

  test('resting view is supplied by the caller, not assumed', () => {
    const s = new HoldStack();
    // Under `tap` the resting view is whichever variant was last selected, so
    // releasing a peek returns to B rather than to A.
    expect(s.resolve('b')).toBe('b');
    s.press('lmb', 'ref');
    expect(s.resolve('b')).toBe('ref');
    s.release('lmb');
    expect(s.resolve('b')).toBe('b');
  });

  test('clear drops everything', () => {
    const s = new HoldStack();
    s.press('a', 'a');
    s.press('b', 'b');
    s.clear();
    expect(s.size).toBe(0);
    expect(s.resolve('ref')).toBe('ref');
  });
});

test.describe('button reconciliation', () => {
  // A mouse reports every button on one pointer id, so keying holds by pointer
  // made the second button replace the first and one release drop both.
  test('mouse holds are keyed by button, touches by pointer', () => {
    expect(holdIdFor('mouse', 1, 0)).not.toBe(holdIdFor('mouse', 1, 2));
    expect(holdIdFor('touch', 1, 0)).not.toBe(holdIdFor('touch', 2, 0));
  });

  // The bitmask and the `button` field disagree on ordering — `button` is
  // 0=left/1=middle/2=right, `buttons` is 1=left/2=right/4=middle.
  test('the bitmask maps to the right buttons', () => {
    expect(diffButtons(0, 1, 1).pressed.map((p) => p.button)).toEqual(['lmb']);
    expect(diffButtons(0, 2, 1).pressed.map((p) => p.button)).toEqual(['rmb']);
    expect(diffButtons(0, 3, 1).pressed.map((p) => p.button)).toEqual(['lmb', 'rmb']);
    // Middle is unbound and must not be invented as either.
    expect(diffButtons(0, 4, 1).pressed).toEqual([]);
  });

  // The sequence Chromium actually delivers: pointerdown(buttons=1), then
  // contextmenu(buttons=3) for the right press — no second pointerdown — then
  // one pointerup(buttons=0) at the end.
  test('a second press and a partial release are seen only in the mask', () => {
    const down = diffButtons(0, 1, 1);
    expect(down.pressed.map((p) => p.button)).toEqual(['lmb']);
    expect(down.released).toEqual([]);

    const second = diffButtons(1, 3, 1);
    expect(second.pressed.map((p) => p.button), 'the right press').toEqual(['rmb']);
    expect(second.released, 'the left button is untouched').toEqual([]);

    const partial = diffButtons(3, 1, 1);
    expect(partial.pressed, 'releasing right presses nothing').toEqual([]);
    expect(partial.released, 'only the right hold goes').toEqual([holdIdFor('mouse', 1, 2)]);

    const last = diffButtons(1, 0, 1);
    expect(last.released).toEqual([holdIdFor('mouse', 1, 0)]);
  });

  test('an unchanged mask is a no-op', () => {
    const d = diffButtons(3, 3, 1);
    expect(d.pressed).toEqual([]);
    expect(d.released).toEqual([]);
  });

  // The full scenario, driven purely through the mask, ending where it started.
  test('mask reconciliation reproduces the ordering scenario', () => {
    const s = new HoldStack();
    const apply = (prev: number, next: number) => {
      const d = diffButtons(prev, next, 1);
      for (const id of d.released) s.release(id);
      for (const { id, button } of d.pressed) {
        s.press(id, viewForPress(button, { mode: 'buttons', isPair: true, half: null }));
      }
    };
    apply(0, 1); // LMB
    expect(s.resolve('ref')).toBe('a');
    apply(1, 3); // + RMB
    expect(s.resolve('ref')).toBe('b');
    apply(3, 1); // - RMB
    expect(s.resolve('ref')).toBe('a');
    apply(1, 0); // - LMB
    expect(s.resolve('ref')).toBe('ref');
  });
});
