import { expect, test } from '@playwright/test';

import { clickBegin, completeProfileAndStart, gotoFresh } from './helpers';

/// Get onto a trial screen with the images decoded.
async function toTrial(page: import('@playwright/test').Page) {
  await gotoFresh(page);
  await clickBegin(page);
  await page.getByRole('button', { name: /^Skip$/ }).click();
  await completeProfileAndStart(page);
  await page.waitForSelector('.trial[data-trial-id]');
  await page.waitForSelector('.viewport:not(.is-loading)');
}

/// Answer the current trial and wait until the *next* one is fully up.
///
/// Waiting on `.viewport:not(.is-loading)` alone is a race: right after the
/// click the outgoing trial is still mounted and already not-loading, so the
/// wait returns instantly and the caller inspects the trial it just answered.
/// The trial id changing is the only reliable edge.
async function advance(page: import('@playwright/test').Page) {
  const before = await page.locator('.trial').getAttribute('data-trial-id');
  await page.locator('.rating-panel button, .pair-panel button').first().click();
  await expect
    .poll(async () => page.locator('.trial').getAttribute('data-trial-id'), { timeout: 15_000 })
    .not.toBe(before);
  await page.waitForSelector('.viewport:not(.is-loading)');
}

/// Advance until a trial of the wanted kind is on screen.
async function toKind(page: import('@playwright/test').Page, kind: 'single' | 'pair') {
  const sel = kind === 'pair' ? '.pair-panel' : '.rating-panel';
  for (let i = 0; i < 60; i++) {
    if (await page.locator(sel).count()) return true;
    await advance(page);
  }
  return false;
}

test.describe('trial input', () => {
  // The point of preloading: every variant is decoded before the observer can
  // answer, so flicking between them is a paint and not a fetch. If a variant
  // were still loading, comparing A to B would mean holding one of them in
  // memory across a network round trip.
  test('every variant is preloaded, so switching is instant', async ({ page }) => {
    await toTrial(page);
    await page.waitForSelector('.viewport.all-ready');

    const layers = await page.evaluate(() =>
      [...document.querySelectorAll<HTMLImageElement>('.viewport img.layer')].map((im) => ({
        layer: im.dataset.layer,
        complete: im.complete,
        decoded: im.naturalWidth > 0,
        shown: im.classList.contains('shown'),
        src: im.src,
      })),
    );

    expect(layers.length).toBeGreaterThanOrEqual(2);
    for (const l of layers) {
      expect(l.decoded, `layer ${l.layer} must be decoded before answering`).toBe(true);
    }
    // Exactly one visible, and `#stimulus` follows it — conditions capture and
    // grading geometry both read that id as "what the observer is looking at".
    expect(layers.filter((l) => l.shown)).toHaveLength(1);
    const stim = await page.evaluate(() => ({
      layer: document.querySelector<HTMLImageElement>('#stimulus')?.dataset.layer,
      count: document.querySelectorAll('#stimulus').length,
    }));
    expect(stim.count).toBe(1);
    expect(stim.layer).toBe(layers.find((l) => l.shown)!.layer);

    // Distinct sources — a stack of the same picture would compare nothing.
    const srcs = new Set(layers.map((l) => l.src));
    expect(srcs.size).toBe(layers.length);
  });

  // Answering before the judged image is painted would record a judgement of
  // something never seen.
  test('the response panel is disabled until the image is painted', async ({ page }) => {
    await gotoFresh(page);
    await clickBegin(page);
    await page.getByRole('button', { name: /^Skip$/ }).click();
    // Hold the images so the loading state is observable rather than a flash.
    let release: () => void = () => {};
    const gate = new Promise<void>((r) => (release = r));
    await page.route('**/api/sources/**', async (route) => {
      await gate;
      await route.continue();
    });
    await completeProfileAndStart(page);
    await page.waitForSelector('.trial[data-trial-id]');

    // The reference is gated, so on a single-stimulus trial the encoding may
    // already be up; assert the invariant that actually matters instead.
    const state = await page.evaluate(() => {
      const btn = document.querySelector<HTMLButtonElement>(
        '.rating-panel button, .pair-panel button',
      );
      const vp = document.querySelector('.viewport');
      return { disabled: btn?.disabled ?? null, loading: vp?.classList.contains('is-loading') };
    });
    if (state.loading) {
      expect(state.disabled, 'cannot answer while the judged image is still loading').toBe(true);
      await expect(page.locator('.viewport-status .spinner')).toBeVisible();
    }

    release();
    await page.waitForSelector('.viewport:not(.is-loading)');
    await expect(
      page.locator('.rating-panel button, .pair-panel button').first(),
    ).toBeEnabled();
  });

  test('arrows cycle the view and space peeks at the original', async ({ page }) => {
    await toTrial(page);
    const shown = () =>
      page.evaluate(
        () => document.querySelector<HTMLImageElement>('.viewport img.shown')!.dataset.layer,
      );

    const first = await shown();
    await page.keyboard.press('ArrowRight');
    expect(await shown(), 'ArrowRight must change the view').not.toBe(first);
    await page.keyboard.press('ArrowLeft');
    expect(await shown(), 'ArrowLeft must come back').toBe(first);

    await page.keyboard.down(' ');
    expect(await shown(), 'holding space shows the original').toBe('ref');
    await expect(page.locator('.trial.revealing')).toHaveCount(1);
    await page.keyboard.up(' ');
    expect(await shown(), 'releasing space returns to the judged image').toBe(first);
  });

  test('number keys rate a single-stimulus trial', async ({ page }) => {
    await toTrial(page);
    expect(await toKind(page, 'single'), 'needed a rating trial').toBe(true);
    const before = await page.locator('.trial').getAttribute('data-trial-id');
    await page.keyboard.press('2');
    // Advancing to a different trial is the observable effect of a recorded
    // response.
    await expect
      .poll(async () => page.locator('.trial').getAttribute('data-trial-id'), { timeout: 10_000 })
      .not.toBe(before);
  });

  test('a/b/c answer a pair trial and digits magnify it', async ({ page }) => {
    await toTrial(page);
    expect(await toKind(page, 'pair'), 'needed a pair trial').toBe(true);

    // Digits are free on pair trials (no 1-4 rating), so they drive the ladder.
    await page.keyboard.press('4');
    await expect(page.locator('.zoom-switch button[data-zoom="4"]')).toHaveClass(/\bon\b/);
    await page.keyboard.press('0');
    await expect(page.locator('.zoom-switch button[data-zoom="1"]')).toHaveClass(/\bon\b/);

    const before = await page.locator('.trial').getAttribute('data-trial-id');
    await page.keyboard.press('c'); // "can't tell"
    await expect
      .poll(async () => page.locator('.trial').getAttribute('data-trial-id'), { timeout: 10_000 })
      .not.toBe(before);
  });

  test('the keyboard help opens and closes', async ({ page }) => {
    await toTrial(page);
    await page.keyboard.press('?');
    await expect(page.locator('.key-help')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.locator('.key-help')).toHaveCount(0);
  });
});

test.describe('hold-to-compare mode', () => {
  // `hold` needs distinct mouse buttons, so it is desktop-only. On the phone
  // projects the picker must not even be offered.
  test('the mode picker is desktop-only', async ({ page }, testInfo) => {
    await toTrial(page);
    const desktop = testInfo.project.name === 'chromium-desktop';
    await expect(page.locator('#input-mode')).toHaveCount(desktop ? 1 : 0);
  });

  test('left button shows A, right shows B, release shows the original', async ({
    page,
  }, testInfo) => {
    test.skip(testInfo.project.name !== 'chromium-desktop', 'hold mode needs a mouse');
    await toTrial(page);
    expect(await toKind(page, 'pair'), 'needed a pair trial').toBe(true);

    await page.locator('#input-mode').selectOption('hold');
    await page.waitForSelector('.trial[data-input-mode="hold"]');
    await page.waitForSelector('.viewport:not(.is-loading)');

    const shown = () =>
      page.evaluate(
        () => document.querySelector<HTMLImageElement>('.viewport img.shown')!.dataset.layer,
      );
    expect(await shown(), 'the original is the resting view in hold mode').toBe('ref');

    const box = (await page.locator('#viewport').boundingBox())!;
    const cx = box.x + box.width / 2;
    const cy = box.y + box.height / 2;

    await page.mouse.move(cx, cy);
    await page.mouse.down({ button: 'left' });
    expect(await shown(), 'holding left shows A').toBe('a');
    await page.mouse.up({ button: 'left' });
    expect(await shown(), 'releasing returns to the original').toBe('ref');

    await page.mouse.down({ button: 'right' });
    expect(await shown(), 'holding right shows B').toBe('b');
    await page.mouse.up({ button: 'right' });
    expect(await shown()).toBe('ref');
  });
});
