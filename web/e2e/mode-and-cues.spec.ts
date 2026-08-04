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
  // The tiled letterbox only shows where the picture does not reach, so it
  // vanishes exactly when someone magnifies — which is most of a careful
  // session, and when knowing "am I on A or B" matters most.
  test('the live variant lights its own edge, and only that edge', async ({ page }) => {
    await toTrial(page);
    expect(await toKind(page, 'pair'), 'needed a pair trial').toBe(true);

    const bars = async () =>
      page.evaluate(() => {
        const g = (sel: string) =>
          getComputedStyle(document.querySelector(sel)!).backgroundImage !== 'none';
        return { left: g('.edge-left'), right: g('.edge-right'), top: g('.edge-top') };
      });

    await page.locator('.view-switch button[data-view="a"]').click();
    expect(await bars(), 'A lights the left edge').toEqual({
      left: true,
      right: false,
      top: false,
    });
    await page.locator('.view-switch button[data-view="b"]').click();
    expect(await bars(), 'B lights the right edge').toEqual({
      left: false,
      right: true,
      top: false,
    });
    await page.locator('.view-switch button[data-view="ref"]').click();
    expect(await bars(), 'the original lights the top edge').toEqual({
      left: false,
      right: false,
      top: true,
    });
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
    const state = await page.evaluate(() => {
      const vp = document.querySelector('#viewport')!.getBoundingClientRect();
      const img = document.querySelector<HTMLImageElement>('#stimulus')!.getBoundingClientRect();
      const bar = document.querySelector('.edge-left')!.getBoundingClientRect();
      return {
        covers: img.width >= vp.width - 1 && img.height >= vp.height - 1,
        // The bar is outside the viewport, so nothing is painted over pixels
        // under judgement — the same rule the reveal hint had to obey.
        barRightOfViewportLeft: bar.right <= vp.left + 1,
        barWidth: bar.width,
      };
    });
    expect(state.covers, 'the stimulus should cover the frame at 8x').toBe(true);
    expect(state.barWidth).toBeGreaterThan(0);
    expect(state.barRightOfViewportLeft, 'the edge must not overlap the stimulus').toBe(true);
  });

  // Same reasoning as the letterbox surround: this is the surround of a
  // psychovisual stimulus, and now the part of it closest to the picture.
  test('the edges are neutral grey, never coloured', async ({ page }) => {
    await toTrial(page);
    const colours = await page.evaluate(() =>
      ['.edge-left', '.edge-right', '.edge-top'].map((s) => {
        const img = getComputedStyle(document.querySelector(s)!).backgroundImage;
        return decodeURIComponent(img);
      }),
    );
    for (const c of colours) {
      // Any fill in the tile must have equal r/g/b — a tint would bias colour
      // judgements right next to the stimulus.
      for (const m of c.matchAll(/#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})/gi)) {
        expect(m[1].toLowerCase(), `tinted edge fill ${m[0]}`).toBe(m[2].toLowerCase());
        expect(m[2].toLowerCase(), `tinted edge fill ${m[0]}`).toBe(m[3].toLowerCase());
      }
    }
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
    await page.keyboard.down('ArrowRight');
    await expect(tie).toHaveClass(/nudge/, { timeout: 30_000 });
    await page.keyboard.up('ArrowRight');

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
