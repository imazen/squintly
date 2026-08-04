import { expect, test, type Page } from '@playwright/test';

import {
  acceptModeChooser,
  clickBegin,
  completeProfileAndStart,
  gotoFresh,
  satisfyGate,
} from './helpers';

/// Walk a fresh visitor as far as the profile's Start button, stopping on
/// whatever comes next.
async function toProfileStart(page: Page) {
  await gotoFresh(page);
  await clickBegin(page);
  await page.getByRole('button', { name: /^Skip$/ }).click();
  await page.getByRole('button', { name: /^room$/ }).click();
  await page.getByRole('button', { name: /^no$/ }).click();
  await page.getByRole('button', { name: /^25-35$/ }).click();
  await page.getByRole('button', { name: /Start rating/i }).click();
}

async function toTrial(page: Page) {
  await gotoFresh(page);
  await clickBegin(page);
  await page.getByRole('button', { name: /^Skip$/ }).click();
  await completeProfileAndStart(page);
  await page.waitForSelector('.trial[data-trial-id]', { timeout: 30_000 });
  await page.waitForSelector('.viewport.all-ready', { timeout: 20_000 });
}

async function toKind(page: Page, kind: 'single' | 'pair'): Promise<boolean> {
  const want = kind === 'single' ? '.rating-panel' : '.pair-panel';
  for (let i = 0; i < 30; i++) {
    if (await page.locator(want).count()) return true;
    const id = await page.locator('.trial').getAttribute('data-trial-id');
    await satisfyGate(page);
    await page.locator('.rating-panel button, .pair-panel button').first().click();
    await expect
      .poll(async () => page.locator('.trial').getAttribute('data-trial-id'), { timeout: 15_000 })
      .not.toBe(id);
    await page.waitForSelector('.viewport.all-ready', { timeout: 20_000 });
  }
  return false;
}

/**
 * One response row from the TSV export, split into fields.
 *
 * Only the trailing NEWLINE is stripped, never `trim()`: `cant_tell_hint_ms` is
 * the last column and is empty on most trials, so the final line legitimately
 * ends in a tab — and `trim()` counts a tab as whitespace, silently turning
 * that empty value into `undefined` and a passing assertion into a wrong one.
 */
async function exportRow(page: Page, trialId: string) {
  const tsv = await page.evaluate(async () => (await fetch('/api/export/responses.tsv')).text());
  const lines = tsv.replace(/\n$/, '').split('\n');
  const head = lines[0].split('\t');
  const col = head.indexOf('trial_id');
  const line = lines.slice(1).find((l) => l.split('\t')[col] === trialId);
  expect(line, `no exported row for ${trialId}`).toBeTruthy();
  return { head, row: line!.split('\t') };
}

const desktop = (t: { project: { name: string } }) => t.project.name === 'chromium-desktop';

test.describe('choosing a comparison mode', () => {
  // The mode changes what the task physically is — whether the reference is
  // what you see at rest, and what your hand does to switch. It used to be
  // picked for people by device class and was reachable only from a dropdown on
  // the trial screen labelled "Interaction mode", so anyone not already fluent
  // in the UI rated a whole session without knowing the alternatives existed.
  test('a first-time observer is stopped before the first trial', async ({ page }, testInfo) => {
    await toProfileStart(page);
    // No trial yet — the mode step comes first, whichever form it takes.
    await expect(page.locator('.trial[data-trial-id]')).toHaveCount(0);
    await expect(page.locator('#mode-continue')).toBeVisible();
    // Exactly one card is marked, so Continue is never ambiguous.
    await expect(page.locator('.mode-card.on')).toHaveCount(1);

    if (desktop(testInfo)) {
      await expect(page.locator('[data-screen="mode-choose"]')).toBeVisible();
      expect(await page.locator('.mode-card').count()).toBeGreaterThan(1);
    } else {
      // Touch drives exactly one mode, and a one-option "choice" is not one —
      // it is a screen asking someone to confirm the only thing that can
      // happen. So it becomes instructions instead.
      await expect(page.locator('[data-screen="mode-howto"]')).toBeVisible();
      await expect(page.locator('.mode-card')).toHaveCount(1);
      await expect(page.locator('.mode-card')).toHaveAttribute('data-mode', 'hold');
    }
  });

  test('the preselected mode is the device default', async ({ page }, testInfo) => {
    await toProfileStart(page);
    const want = desktop(testInfo) ? 'buttons' : 'hold';
    await expect(page.locator('.mode-card.on')).toHaveAttribute('data-mode', want);
  });

  // `tap` asks for three ~44px targets below the picture and a look away from
  // the stimulus per switch, on the device where the picture is smallest.
  test('touch is offered hold and nothing else', async ({ page }, testInfo) => {
    test.skip(desktop(testInfo), 'this device has a mouse');
    await toProfileStart(page);
    await expect(page.locator('.mode-card[data-mode="tap"]')).toHaveCount(0);
    await expect(page.locator('.mode-card[data-mode="buttons"]')).toHaveCount(0);
    await acceptModeChooser(page);
    await page.waitForSelector('.trial[data-input-mode="hold"]', { timeout: 30_000 });
    // And no dead one-option control on the trial screen either.
    await expect(page.locator('#input-mode')).toHaveCount(0);
  });

  // A phone that picked `tap` before it was withdrawn must not be stranded in a
  // UI the device no longer offers.
  test('a stored mode the device no longer offers is not honoured', async ({ page }, testInfo) => {
    test.skip(desktop(testInfo), 'this device still offers tap');
    await toTrial(page);
    await page.evaluate(() => localStorage.setItem('squintly_input_mode', 'tap'));
    await page.goto('/');
    await page.waitForSelector('.trial[data-trial-id]', { timeout: 30_000 });
    expect(
      await page.evaluate(() => document.querySelector<HTMLElement>('.trial')!.dataset.inputMode),
    ).toBe('hold');
  });

  // This is the one screen where a how-to is read, because it is about the
  // gesture you are about to use. Naming the actual hand movement is the point.
  test('every card says what the hand does', async ({ page }) => {
    await toProfileStart(page);
    for (const card of await page.locator('.mode-card').all()) {
      const mode = await card.getAttribute('data-mode');
      await expect(card.locator('.mode-card-how li')).not.toHaveCount(0);
      const text = (await card.innerText()).toLowerCase();
      if (mode === 'buttons') expect(text).toMatch(/left mouse button/);
      if (mode === 'hold') expect(text).toMatch(/left half/);
      if (mode === 'tap') expect(text).toMatch(/tap/);
    }
  });

  test('the choice is applied to the trial and remembered', async ({ page }, testInfo) => {
    test.skip(!desktop(testInfo), 'touch offers only one mode, so there is nothing to switch to');
    await toProfileStart(page);
    await page.locator('.mode-card[data-mode="tap"]').click();
    await page.locator('#mode-continue').click();
    await page.waitForSelector('.trial[data-input-mode="tap"]', { timeout: 30_000 });
    expect(await page.evaluate(() => localStorage.getItem('squintly_input_mode'))).toBe('tap');
  });

  // Once, not every load. A device default silently applied is not a choice,
  // which is why "has chosen" is tracked separately from "which mode".
  test('it is not asked again on the next visit', async ({ page }) => {
    await toTrial(page);
    await page.goto('/');
    await page.waitForSelector('.trial[data-trial-id]', { timeout: 30_000 });
    await expect(page.locator('#mode-continue')).toHaveCount(0);
  });

  // The observers already in the study were never asked. They should get the
  // prompt on their next visit rather than never — so the flag is about having
  // chosen, not about having a mode.
  test('an observer who never chose is asked on their next visit', async ({ page }) => {
    await toTrial(page);
    await page.evaluate(() => localStorage.removeItem('squintly_input_mode'));
    await page.goto('/');
    await expect(page.locator('#mode-continue')).toBeVisible();
    await acceptModeChooser(page);
    await page.waitForSelector('.trial[data-trial-id]', { timeout: 30_000 });
  });
});

test.describe('the edge indicator', () => {
  /// Background colour of each bar, as computed rgb.
  const bars = (page: Page) =>
    page.evaluate(() => {
      const c = (sel: string) =>
        getComputedStyle(document.querySelector(sel)!).backgroundColor;
      return { left: c('.edge-left'), right: c('.edge-right'), top: c('.edge-top') };
    });

  const ACCENT = 'rgb(74, 209, 255)'; // --accent, what an active A/B button takes
  const GOOD = 'rgb(123, 229, 138)'; // --good, what an active Original takes
  const DARK = 'rgb(20, 20, 24)'; // unlit

  // The tiled letterbox only shows where the picture does not reach, so it
  // vanishes exactly when someone magnifies — which is most of a careful
  // session, and when knowing "am I on A or B" matters most.
  test('the live variant lights its own edge, and only that edge', async ({ page }) => {
    await toTrial(page);
    expect(await toKind(page, 'pair'), 'needed a pair trial').toBe(true);

    await page.locator('.view-switch button[data-view="a"]').click();
    expect(await bars(page), 'A lights the left edge').toEqual({
      left: ACCENT,
      right: DARK,
      top: DARK,
    });
    await page.locator('.view-switch button[data-view="b"]').click();
    expect(await bars(page), 'B lights the right edge').toEqual({
      left: DARK,
      right: ACCENT,
      top: DARK,
    });
    await page.locator('.view-switch button[data-view="ref"]').click();
    expect(await bars(page), 'the original lights the top edge').toEqual({
      left: DARK,
      right: DARK,
      top: GOOD,
    });
  });

  // The first version obeyed the letterbox's neutral-grey rule and was
  // invisible on a phone. The colour is the whole point now: it is the same
  // colour that arm's button takes when active, so the two cues agree.
  test('a lit edge wears its own button colour', async ({ page }) => {
    await toTrial(page);
    expect(await toKind(page, 'pair'), 'needed a pair trial').toBe(true);
    for (const [view, want] of [
      ['a', ACCENT],
      ['b', ACCENT],
      ['ref', GOOD],
    ] as const) {
      await page.locator(`.view-switch button[data-view="${view}"]`).click();
      const btn = await page.evaluate(
        (v) =>
          getComputedStyle(document.querySelector(`.view-switch button[data-view="${v}"]`)!)
            .backgroundColor,
        view,
      );
      expect(btn, `the ${view} button should be lit`).toBe(want);
      const bar = (await bars(page))[view === 'a' ? 'left' : view === 'b' ? 'right' : 'top'];
      expect(bar, `the ${view} edge should match its button`).toBe(want);
    }
  });

  // Screen space is the binding constraint on a phone. The bars must be no
  // wider than the letter they carry, and must reach the screen edge — a page
  // gutter outside them would waste exactly what making them narrow saved.
  test('the bars are thin and reach the screen edge', async ({ page }) => {
    await toTrial(page);
    const m = await page.evaluate(() => {
      const l = document.querySelector('.edge-left')!.getBoundingClientRect();
      const r = document.querySelector('.edge-right')!.getBoundingClientRect();
      const t = document.querySelector('.edge-top')!.getBoundingClientRect();
      const vp = document.querySelector('#viewport')!.getBoundingClientRect();
      return {
        lw: l.width,
        rw: r.width,
        th: t.height,
        leftGap: l.left,
        rightGap: document.documentElement.clientWidth - r.right,
        // Adjacent to the picture, never on top of it.
        touchesViewportLeft: Math.abs(l.right - vp.left) < 1,
        touchesViewportRight: Math.abs(r.left - vp.right) < 1,
      };
    });
    expect(m.lw, 'left bar under 8px').toBeLessThan(8);
    expect(m.rw, 'right bar under 8px').toBeLessThan(8);
    expect(m.lw, 'wide enough for the letter').toBeGreaterThan(3);
    expect(m.th, 'the top bar is thinner still — vertical space is scarcer').toBeLessThan(8);
    expect(m.leftGap, 'no gutter outside the left bar').toBeLessThan(1);
    expect(m.rightGap, 'no gutter outside the right bar').toBeLessThan(1);
    expect(m.touchesViewportLeft).toBe(true);
    expect(m.touchesViewportRight).toBe(true);
  });

  // The reason this exists at all: it has to survive the stimulus covering the
  // whole frame, which is what killed the letterbox cue.
  test('it stays visible when the picture covers the frame', async ({ page }) => {
    await toTrial(page);
    // Digits are only free for the ladder on a pair trial; on a single, 1-4 are
    // the rating.
    expect(await toKind(page, 'pair'), 'needed a pair trial').toBe(true);
    // Magnify hard, so the stimulus is larger than the viewport in both axes.
    await page.keyboard.press('8');
    await expect(page.locator('#zoom-readout')).toHaveText('8×');
    await page.locator('.view-switch button[data-view="a"]').click();
    const state = await page.evaluate(() => {
      const vp = document.querySelector('#viewport')!.getBoundingClientRect();
      const img = document.querySelector<HTMLImageElement>('#stimulus')!.getBoundingClientRect();
      const bar = document.querySelector('.edge-left')!.getBoundingClientRect();
      return {
        covers: img.width >= vp.width - 1 && img.height >= vp.height - 1,
        // Outside the viewport, so nothing is painted over pixels under
        // judgement — the same rule the reveal hint had to obey.
        outsideViewport: bar.right <= vp.left + 1,
      };
    });
    expect(state.covers, 'the stimulus should cover the frame at 8x').toBe(true);
    expect(state.outsideViewport, 'the edge must not overlap the stimulus').toBe(true);
    expect((await bars(page)).left, 'and it is still lit').toBe(ACCENT);
  });
});

test.describe('suggesting "can\'t tell"', () => {
  // At threshold the truthful answer is a tie, but the button reads as giving
  // up — so people grind on and eventually guess, and a guess recorded as a
  // preference is worse data than a recorded tie.
  //
  // Deliberately holds for real rather than mocking the clock: the thing under
  // test is that *held* time accrues, which a faked timer would assert nothing
  // about.
  test('a long comparison offers the tie, and the offer is recorded', async ({ page }) => {
    test.setTimeout(90_000);
    await toTrial(page);
    expect(await toKind(page, 'pair'), 'needed a pair trial').toBe(true);
    const id = await page.locator('.trial').getAttribute('data-trial-id');
    await satisfyGate(page);

    const tie = page.locator('.pair-panel button[data-c="tie"]');
    await expect(tie, 'nothing suggested before any real comparison').not.toHaveClass(/nudge/);

    // Hold a variant up past the threshold. Keys hold exactly like the pointer.
    const restFill = await tie.evaluate((el) => getComputedStyle(el).backgroundColor);
    await page.keyboard.down('ArrowRight');
    await expect(tie).toHaveClass(/nudge/, { timeout: 30_000 });
    await page.keyboard.up('ArrowRight');

    // The FILL has to move, not just a 1px outline — an inset border alone was
    // too small a change to notice while the eye is on the picture, which made
    // the hint useless. Sampled repeatedly because it is a slow cycle and the
    // trough matches the resting colour by design.
    await expect
      .poll(async () => tie.evaluate((el) => getComputedStyle(el).backgroundColor), {
        timeout: 5_000,
        intervals: [100],
      })
      .not.toBe(restFill);

    await tie.click();
    await expect
      .poll(async () => page.locator('.trial').getAttribute('data-trial-id'), { timeout: 15_000 })
      .not.toBe(id);

    const { head, row } = await exportRow(page, id!);
    const cell = (n: string) => row[head.indexOf(n)];
    expect(cell('choice')).toBe('tie');
    // Without this column a hinted tie is indistinguishable from a spontaneous
    // one, and the hint fires on exactly the hardest trials.
    expect(Number(cell('cant_tell_hint_ms')), 'the hint must be recorded').toBeGreaterThan(0);
  });

  // The great majority of trials must carry no hint, or the column says
  // nothing and the nudge is firing on ordinary care rather than on the tail.
  test('a quick answer records no hint', async ({ page }) => {
    await toTrial(page);
    expect(await toKind(page, 'pair'), 'needed a pair trial').toBe(true);
    const id = await page.locator('.trial').getAttribute('data-trial-id');
    await satisfyGate(page);
    await page.locator('.pair-panel button[data-c="a"]').click();
    await expect
      .poll(async () => page.locator('.trial').getAttribute('data-trial-id'), { timeout: 15_000 })
      .not.toBe(id);

    const { head, row } = await exportRow(page, id!);
    expect(row[head.indexOf('cant_tell_hint_ms')]).toBe('');
  });

  // A single-stimulus trial has no tie to offer, so the ticker must not run
  // looking for a button that is not there.
  test('a single-stimulus trial is never nudged', async ({ page }) => {
    await toTrial(page);
    expect(await toKind(page, 'single'), 'needed a rating trial').toBe(true);
    await satisfyGate(page);
    await page.keyboard.down('ArrowRight');
    await page.waitForTimeout(2_000);
    await page.keyboard.up('ArrowRight');
    await expect(page.locator('.rating-panel button.nudge')).toHaveCount(0);
  });
});

test.describe('the how-to pill', () => {
  // The gesture is learned in a trial or two; after that the pill is a
  // permanent band of text beside the picture, on the screen with the least
  // room for one.
  test('it can be dismissed, and stays dismissed across trials', async ({ page }) => {
    await toTrial(page);
    // The pill is suppressed while the gate hint is up, so open the gate first
    // — otherwise this is asserting on a state where it is correctly absent.
    await satisfyGate(page);
    const hint = page.locator('#hint');
    await expect(hint).toBeVisible();

    await page.locator('#hint-dismiss').click();
    await expect(hint).toBeHidden();

    // Not just this trial.
    const id = await page.locator('.trial').getAttribute('data-trial-id');
    await page.locator('.rating-panel button, .pair-panel button').first().click();
    await expect
      .poll(async () => page.locator('.trial').getAttribute('data-trial-id'), { timeout: 15_000 })
      .not.toBe(id);
    await expect(page.locator('#hint')).toBeHidden();

    // Nor just this load. localStorage, deliberately not the database — it is a
    // preference about chrome and says nothing about a judgement.
    expect(await page.evaluate(() => localStorage.getItem('squintly_hint_dismissed'))).toBe('1');
    await page.goto('/');
    await page.waitForSelector('.trial[data-trial-id]', { timeout: 30_000 });
    await expect(page.locator('#hint')).toBeHidden();
  });

  // It used to turn full accent blue whenever the original was up — the
  // loudest thing on screen, next to a stimulus under judgement. The lit edge
  // bar carries that signal in colour now.
  test('showing the original no longer turns the pill bright blue', async ({ page }) => {
    await toTrial(page);
    await page.locator('.view-switch button[data-view="ref"]').click();
    await expect(page.locator('.trial.revealing')).toHaveCount(1);
    const colour = await page
      .locator('#hint')
      .evaluate((el) => getComputedStyle(el).color);
    expect(colour, 'the hint must not wear the accent').not.toBe('rgb(74, 209, 255)');
  });
});

test.describe('the identifier panel', () => {
  // "The B one with the green band" is not a bug report; an encoding id is.
  // Someone who meets a corrupt encode or an inexplicable artefact needs to be
  // able to say exactly which image they mean.
  test('names every arm on screen, with ids that can be looked up', async ({ page }) => {
    await toTrial(page);
    expect(await toKind(page, 'pair'), 'needed a pair trial').toBe(true);
    const id = await page.locator('.trial').getAttribute('data-trial-id');

    await page.locator('#info-btn').click();
    const panel = page.locator('.info-help');
    await expect(panel).toBeVisible();

    const text = await panel.innerText();
    // The trial itself, so a report can be located in the data.
    expect(text).toContain(id!);
    // Both arms, each identified rather than described.
    for (const arm of ['A encoding', 'B encoding', 'A codec', 'B codec']) {
      expect(text, `missing ${arm}`).toContain(arm);
    }
    // And the original, which is what the artefact is being judged against.
    expect(text).toContain('source sha256');
    expect(text).toContain('original url');
    // Attributable to a version — otherwise a fixed bug cannot be told from a
    // live one.
    expect(text).toMatch(/build/);

    await page.keyboard.press('Escape');
    await expect(panel).toHaveCount(0);
  });

  test('the i key opens it too, and copying yields the full record', async ({ page, context }) => {
    await context.grantPermissions(['clipboard-read', 'clipboard-write']);
    await toTrial(page);
    await page.keyboard.press('i');
    await expect(page.locator('.info-help')).toBeVisible();

    await page.locator('#info-copy').click();
    await expect(page.locator('#info-copy')).toHaveText(/Copied/);
    const clip = await page.evaluate(() => navigator.clipboard.readText());
    // `label: value` lines — pasteable into an issue as-is.
    expect(clip).toMatch(/^trial: /m);
    expect(clip).toMatch(/^A encoding: /m);
    expect(clip).toMatch(/^source sha256: /m);
    // The whole record, not just what happened to be visible.
    expect(clip.split('\n').length).toBeGreaterThan(8);
  });

  // A single-stimulus trial has no B, so it must not invent one.
  test('a single-stimulus trial lists one arm', async ({ page }) => {
    await toTrial(page);
    expect(await toKind(page, 'single'), 'needed a rating trial').toBe(true);
    await page.locator('#info-btn').click();
    const text = await page.locator('.info-help').innerText();
    expect(text).toContain('A encoding');
    expect(text).not.toContain('B encoding');
  });
});

// Two paragraphs explaining the same gesture, one line apart, on the screen
// with the least room for either.
test.describe('one hint at a time', () => {
  test('the how-to pill is suppressed while the gate hint is up', async ({ page }) => {
    await toTrial(page);
    expect(await toKind(page, 'pair'), 'needed a pair trial').toBe(true);

    // Gated: the gate hint carries the gesture, and the pill must not repeat
    // it. The pill may still be up for "drag to explore", which the gate hint
    // never says — what must not happen is the same sentence twice.
    await expect(page.locator('#panel')).toHaveAttribute('data-gated', 'yes');
    await expect(page.locator('#gate-hint')).toBeVisible();
    const gated = await page.locator('#hint').innerText().catch(() => '');
    expect(gated, 'the pill must not repeat the gesture').not.toMatch(/hold the (left|right)/i);

    // Gate opens: the gate hint goes, the pill comes back.
    await satisfyGate(page);
    await expect(page.locator('#gate-hint')).toBeHidden();
    await expect(page.locator('#hint')).toBeVisible();
    // And it is the one that can be dismissed.
    await expect(page.locator('#hint-dismiss')).toBeVisible();
  });
})

test.describe('trial chrome on a phone', () => {
  // The control row wrapped to two lines on a narrow screen, costing more
  // vertical space than everything in it is worth.
  test('the controls fit on one line', async ({ page }, testInfo) => {
    await toTrial(page);
    const m = await page.evaluate(() => {
      const bar = document.querySelector('.trial-controls')!;
      const kids = [...bar.children].filter((c) => (c as HTMLElement).offsetParent !== null);
      const rects = kids.map((c) => c.getBoundingClientRect());
      const mids = rects.map((r) => r.top + r.height / 2);
      return {
        barH: bar.getBoundingClientRect().height,
        tallest: Math.max(...rects.map((r) => r.height)),
        // Distinct tops do NOT mean wrapping — children of different heights on
        // one line start at different y. The centreline is what tells them
        // apart, and a ragged one is the thing that reads as a wrapped row.
        midSpread: Math.max(...mids) - Math.min(...mids),
        widthUsed: rects.reduce((a, r) => a + r.width, 0),
        barW: bar.getBoundingClientRect().width,
      };
    });
    // One line: the row is no taller than its tallest child.
    expect(m.barH, 'the control row must not wrap').toBeLessThan(m.tallest + 4);
    expect(m.widthUsed, 'and its contents must fit across').toBeLessThan(m.barW);
    // Only asserted where it is the binding constraint. A desktop has room and
    // more controls (mode picker, keyboard sheet, zoom stepper).
    if (desktop(testInfo)) return;
    expect(m.midSpread, 'every control shares a centreline').toBeLessThan(1.5);
  });

  // Meaningless without a keyboard, and the pause menu carries it anyway.
  test('the keyboard cheatsheet button is hidden on touch', async ({ page }, testInfo) => {
    await toTrial(page);
    const visible = await page
      .locator('#keys-btn')
      .evaluate((el) => (el as HTMLElement).offsetParent !== null);
    expect(visible).toBe(desktop(testInfo));
  });

  // Pinch and double-tap both cover magnification on touch, so the -/+ pair is
  // redundant chrome on the device with the least room for it. The readout
  // stays: the factor is part of what is being judged.
  test('the zoom stepper is dropped on touch but the factor is not', async ({ page }, testInfo) => {
    await toTrial(page);
    await expect(page.locator('#zoom-readout')).toBeVisible();
    const steppers = await page
      .locator('.zoom-switch button')
      .evaluateAll((els) => els.filter((e) => (e as HTMLElement).offsetParent !== null).length);
    expect(steppers).toBe(desktop(testInfo) ? 2 : 0);
  });

  // It floated off-centre between two 44px buttons.
  test('the magnification readout is centred against the stepper', async ({ page }, testInfo) => {
    test.skip(!desktop(testInfo), 'the stepper is only rendered where there is a mouse');
    await toTrial(page);
    const m = await page.evaluate(() => {
      const out = document.querySelector('#zoom-readout')!.getBoundingClientRect();
      const btn = document.querySelector('.zoom-switch button')!.getBoundingClientRect();
      return { outMid: out.top + out.height / 2, btnMid: btn.top + btn.height / 2 };
    });
    expect(Math.abs(m.outMid - m.btnMid), 'readout must share the stepper centreline').toBeLessThan(
      1.5,
    );
  });

  // "imazen26-7000-lilith-plots · Operator's own work" spent a scarce line on a
  // licence string nobody reads mid-trial. Attribution still ships — the
  // credits panel and the `i` panel both carry it — so the header keeps only
  // the corpus, which is the part that identifies the picture.
  test('the header names the corpus, not the licence', async ({ page }) => {
    await toTrial(page);
    const label = page.locator('.trial-license');
    await expect(label).toBeVisible();
    const text = await label.innerText();
    expect(text).not.toMatch(/own work|CC[- ]BY|public domain/i);
    // Still recoverable without leaving the trial.
    expect(await label.getAttribute('title')).toMatch(/·/);
    await page.locator('#info-btn').click();
    expect(await page.locator('.info-help').innerText()).toContain('license');
  });
});

test.describe('measuring a card on a phone', () => {
  // A card is 85.6mm on its long edge; a phone is about 65mm wide. So a
  // landscape card physically cannot fit across a portrait screen — the slider
  // ran out of travel before the rectangle reached a real card, which made
  // calibration impossible on the device this study mostly runs on.
  test('the card lies upright where the screen is taller than it is wide', async ({ page }) => {
    await gotoFresh(page);
    await clickBegin(page);
    await expect(page.locator('#slider')).toBeVisible();

    const box = await page.locator('#card').boundingBox();
    const portraitScreen = await page.evaluate(() => window.innerHeight > window.innerWidth);
    expect(
      box!.height > box!.width,
      'a portrait screen must start with the card turned',
    ).toBe(portraitScreen);
  });

  test('the card can be turned either way, and the measurement is unchanged', async ({ page }) => {
    await gotoFresh(page);
    await clickBegin(page);
    await page.locator('#slider').fill('400');
    await page.locator('#slider').dispatchEvent('input');

    const dims = async () => {
      const b = (await page.locator('#card').boundingBox())!;
      return { w: Math.round(b.width), h: Math.round(b.height) };
    };
    const before = await dims();
    await page.locator('#rotate-card').click();
    const after = await dims();
    // Same rectangle, turned: the long edge is always the slider's value, so
    // the measurement is identical either way. CSS pixels are square.
    expect(after.w).toBe(before.h);
    expect(after.h).toBe(before.w);
    expect(Math.max(after.w, after.h), 'the long edge is what the slider sets').toBe(400);

    // And it stores the same mm-per-px whichever way up it was measured.
    await page.getByRole('button', { name: /Looks right/i }).click();
    await page.getByRole('button', { name: /^Skip$/ }).click();
    const stored = await page.evaluate(() =>
      JSON.parse(localStorage.getItem('squintly:calibration') || 'null'),
    );
    expect(stored.css_px_per_mm).toBeCloseTo(400 / 85.6, 3);
  });
});

test.describe('the board reports effort honestly', () => {
  // The swap median read 0 for everyone. Responses written before migration
  // 0019 carry the column's NOT NULL DEFAULT 0, which means "never recorded",
  // not "never switched" — and with 91 of the first 154 live responses
  // predating it, the median landed squarely in the backfill (measured
  // 2026-08-04; the same rows median 69 once excluded).
  test('swaps come from instrumented trials, and hours are engaged time', async ({ page }) => {
    await toTrial(page);
    // Do some real comparing so there is something to measure.
    for (let i = 0; i < 2; i++) {
      await satisfyGate(page);
      await page.keyboard.down('ArrowRight');
      await page.keyboard.up('ArrowRight');
      await page.keyboard.down('ArrowLeft');
      await page.keyboard.up('ArrowLeft');
      const id = await page.locator('.trial').getAttribute('data-trial-id');
      await page.locator('.rating-panel button, .pair-panel button').first().click();
      await expect
        .poll(async () => page.locator('.trial').getAttribute('data-trial-id'), { timeout: 15_000 })
        .not.toBe(id);
      await page.waitForSelector('.viewport.all-ready', { timeout: 20_000 });
    }

    const board = await page.evaluate(async () => (await fetch('/api/leaderboard')).json());
    expect(Array.isArray(board)).toBe(true);
    const mine = board.find((r: { trials: number }) => r.trials > 0);
    expect(mine, 'this observer should be on the board').toBeTruthy();

    // The counter was exercised, so it must not read as zero.
    expect(mine.instrumented_trials, 'these trials carry the counter').toBeGreaterThan(0);
    expect(mine.median_switches, 'and the median must reflect them').toBeGreaterThan(0);

    // Engaged time: real, and bounded by wall-clock — a measure that can exceed
    // the elapsed session is not a measure of time spent.
    expect(mine.active_seconds).toBeGreaterThan(0);
    expect(mine.active_seconds, 'engaged time cannot exceed the session').toBeLessThan(600);
  });
});

test.describe('images come from the store', () => {
  // Proxying every trial image made the server pay for the bytes twice — a real
  // source is 9.5 MB — and added a round trip to the thing the observer is
  // waiting for. The proxy exists for the canvas paths (R2 serves the corpus
  // without CORS), and plain <img> display needs none of that.
  test('trial images are fetched direct, not through the proxy', async ({ page }) => {
    await toTrial(page);
    const urls = await page.evaluate(() =>
      [...document.querySelectorAll<HTMLImageElement>('.viewport img.layer')].map((i) => i.src),
    );
    expect(urls.length).toBeGreaterThanOrEqual(2);
    for (const u of urls) {
      expect(u, `${u} should not be proxied`).not.toContain('/api/proxy/');
    }
  });

  // A sha256 identifies an image; it does not say what it is. Every imazen26
  // source carries a meaningful filename, and it is what a person uses to find
  // the picture again when reporting a bad encode.
  test('the identifier panel lists the source filename', async ({ page }) => {
    await toTrial(page);
    await page.locator('#info-btn').click();
    const text = await page.locator('.info-help').innerText();
    expect(text).toContain('source file');
    expect(text).toMatch(/source file\s+\S+/);
  });
});
