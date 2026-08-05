import { expect, test } from './fixtures';
import { type Page } from '@playwright/test';

import { clickBegin, completeProfileAndStart, gotoFresh, satisfyGate } from './helpers';

/// Walk a fresh visitor into trials.
async function toTrial(page: Page) {
  await gotoFresh(page);
  await clickBegin(page);
  await page.getByRole('button', { name: /^Skip$/ }).click();
  await completeProfileAndStart(page);
  await page.waitForSelector('.trial[data-trial-id]', { timeout: 30_000 });
  await page.waitForSelector('.viewport.all-ready', { timeout: 20_000 });
}

/// Draw trials until one of the requested kind appears.
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

/// Which mode this project's device landed in — it decides what is on screen
/// at rest, and therefore what the gate is still waiting for.
async function mode(page: Page): Promise<string> {
  return (await page.locator('.trial').getAttribute('data-input-mode')) ?? 'tap';
}

test.describe('the seen-both gate', () => {
  // Rating a pair you have only half-looked at records an opinion about an
  // image that was never on screen. The panel is dark by default, and only the
  // arm you have actually viewed counts.
  test('a pair cannot be answered until both arms have been viewed', async ({ page }) => {
    await toTrial(page);
    expect(await toKind(page, 'pair'), 'needed a pair trial').toBe(true);

    await expect(page.locator('#panel')).toHaveAttribute('data-gated', 'yes');
    await expect(page.locator('#gate-hint')).toBeVisible();
    for (const b of await page.locator('.pair-panel button').all()) {
      await expect(b).toBeDisabled();
    }

    // Under `tap` A is the resting view, so only B is outstanding; under
    // `hold`/`buttons` the reference rests and both arms are.
    const resting = await mode(page);
    await expect(page.locator('#gate-hint')).toContainText(
      resting === 'tap' ? /\bB\b/ : /A and B/,
    );

    await page.locator('.view-switch button[data-view="a"]').click();
    await page.locator('.view-switch button[data-view="b"]').click();
    await expect(page.locator('#panel')).toHaveAttribute('data-gated', 'no');
    await expect(page.locator('#gate-hint')).toBeHidden();
    for (const b of await page.locator('.pair-panel button').all()) {
      await expect(b).toBeEnabled();
    }
  });

  // The hint has to be actionable in the mode the observer is actually in.
  // "look at B first" names an A/B control that only `tap` puts on screen; in
  // `hold` you press a half of the frame, so the same sentence would leave
  // someone staring at a dead panel with no idea what it wants.
  test('the hint describes the gesture this mode actually uses', async ({ page }) => {
    await toTrial(page);
    expect(await toKind(page, 'pair'), 'needed a pair trial').toBe(true);
    const hint = page.locator('#gate-hint');
    switch (await mode(page)) {
      case 'tap':
        await expect(hint).toContainText(/look at/i);
        break;
      case 'hold':
        await expect(hint).toContainText(/press and hold the left and right half/i);
        break;
      case 'buttons':
        await expect(hint).toContainText(/hold the left and the right button/i);
        break;
    }
  });

  // The buttons are only half the enforcement: keys reach `commit` directly, so
  // a gated keypress has to be a no-op rather than a disabled-looking button
  // that still answers.
  test('the keyboard cannot bypass the gate', async ({ page }) => {
    await toTrial(page);
    expect(await toKind(page, 'pair'), 'needed a pair trial').toBe(true);
    await expect(page.locator('#panel')).toHaveAttribute('data-gated', 'yes');

    const before = await page.locator('.trial').getAttribute('data-trial-id');
    await page.keyboard.press('a');
    await page.waitForTimeout(600);
    expect(
      await page.locator('.trial').getAttribute('data-trial-id'),
      'a gated keypress must not answer',
    ).toBe(before);

    // Holding the arrows is looking, so the same key then works.
    await page.keyboard.press('ArrowLeft');
    await page.keyboard.press('ArrowRight');
    await expect(page.locator('#panel')).toHaveAttribute('data-gated', 'no');
    await page.keyboard.press('a');
    await expect
      .poll(async () => page.locator('.trial').getAttribute('data-trial-id'), { timeout: 15_000 })
      .not.toBe(before);
  });

  // The gate asks for the arm being *judged*, not for ceremony. On a single
  // trial that is the compressed image — already at rest under `tap` (so no
  // hoop at all), and one press away under `hold`, where the reference rests
  // and the thing being rated is genuinely not on screen yet.
  test('a single-stimulus trial asks only for the image being rated', async ({ page }) => {
    await toTrial(page);
    expect(await toKind(page, 'single'), 'needed a rating trial').toBe(true);

    if ((await mode(page)) === 'tap') {
      await expect(page.locator('#panel')).toHaveAttribute('data-gated', 'no');
      return;
    }
    await expect(page.locator('#panel')).toHaveAttribute('data-gated', 'yes');
    // Either wording, but never one naming a half or an arm: on a single trial
    // there is no left/right split and no B, so "hold the left button to see A"
    // sends someone to press a side for something that does not exist.
    await expect(page.locator('#gate-hint')).toContainText(
      /(press and hold|hold any mouse button) to see the compressed image first/i,
    );
    await page.locator('.view-switch button[data-view="a"]').click();
    await expect(page.locator('#panel')).toHaveAttribute('data-gated', 'no');
  });
});

test.describe('taking back an answer', () => {
  // The trial screen is driven by thumb-sized buttons and single keystrokes, so
  // a stray tap is easy and used to be permanent.
  // On the first trial of a session there is nothing behind you, so the
  // control must not be sitting there inviting a press that does nothing.
  test('there is nothing to take back on the first trial', async ({ page }) => {
    await toTrial(page);
    await expect(page.locator('#undo-btn')).toBeHidden();
  });

  test('undo reopens the previous trial and re-gates it', async ({ page }) => {
    await toTrial(page);
    expect(await toKind(page, 'pair'), 'needed a pair trial').toBe(true);

    const first = await page.locator('.trial').getAttribute('data-trial-id');
    await satisfyGate(page);
    await page.locator('.pair-panel button[data-c="a"]').click();
    await expect
      .poll(async () => page.locator('.trial').getAttribute('data-trial-id'), { timeout: 15_000 })
      .not.toBe(first);

    await expect(page.locator('#undo-btn')).toBeVisible();
    await page.locator('#undo-btn').click();

    // Back on the trial we just answered...
    await expect
      .poll(async () => page.locator('.trial').getAttribute('data-trial-id'), { timeout: 15_000 })
      .toBe(first);
    // ...with the gate closed again: an undo is for a misclick, not for
    // changing an answer without re-examining the images.
    await expect(page.locator('#panel')).toHaveAttribute('data-gated', 'yes');
    // And it is a one-shot; there is no walking back through the whole session.
    await expect(page.locator('#undo-btn')).toBeHidden();
  });

  test('the u key does the same thing', async ({ page }) => {
    await toTrial(page);
    const first = await page.locator('.trial').getAttribute('data-trial-id');
    await satisfyGate(page);
    await page.locator('.rating-panel button, .pair-panel button').first().click();
    await expect
      .poll(async () => page.locator('.trial').getAttribute('data-trial-id'), { timeout: 15_000 })
      .not.toBe(first);

    await page.keyboard.press('u');
    await expect
      .poll(async () => page.locator('.trial').getAttribute('data-trial-id'), { timeout: 15_000 })
      .toBe(first);
  });

  // The revision has to reach the server, or "undo" is a lie told to the
  // observer while the original answer stands in the data.
  test('the corrected answer replaces the original in the export', async ({ page }) => {
    await toTrial(page);
    expect(await toKind(page, 'pair'), 'needed a pair trial').toBe(true);
    const id = await page.locator('.trial').getAttribute('data-trial-id');

    await satisfyGate(page);
    await page.locator('.pair-panel button[data-c="a"]').click();
    await expect
      .poll(async () => page.locator('.trial').getAttribute('data-trial-id'), { timeout: 15_000 })
      .not.toBe(id);

    await page.locator('#undo-btn').click();
    await expect
      .poll(async () => page.locator('.trial').getAttribute('data-trial-id'), { timeout: 15_000 })
      .toBe(id);
    await satisfyGate(page);
    await page.locator('.pair-panel button[data-c="b"]').click();
    await expect
      .poll(async () => page.locator('.trial').getAttribute('data-trial-id'), { timeout: 15_000 })
      .not.toBe(id);

    const tsv = await page.evaluate(async () => (await fetch('/api/export/responses.tsv')).text());
    const lines = tsv.trim().split('\n');
    const head = lines[0].split('\t');
    const row = lines.slice(1).find((l) => l.split('\t')[head.indexOf('trial_id')] === id);
    expect(row, `no exported row for trial ${id}`).toBeTruthy();
    const cell = (name: string) => row!.split('\t')[head.indexOf(name)];
    // The answer that counts is the corrected one...
    expect(cell('choice')).toBe('b');
    // ...and the first one is kept, because "changed their mind" is a fact
    // about the session that analysis may want.
    expect(cell('original_choice')).toBe('a');
    expect(Number(cell('revision_count'))).toBe(1);
    expect(Number(cell('revised_at'))).toBeGreaterThan(0);
  });
});
