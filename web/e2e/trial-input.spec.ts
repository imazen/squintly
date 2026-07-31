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
  // Splitting by half rather than by mouse button is what makes this work with
  // a thumb, so it must be offered on every device — including the phone
  // projects, which are the ones the study actually runs on.
  test('the mode is offered on every device', async ({ page }) => {
    await toTrial(page);
    await expect(page.locator('#input-mode')).toHaveCount(1);
    await expect(page.locator('#input-mode option[value="hold"]')).toHaveCount(1);
  });

  test('left half shows A, right half shows B, release shows the original', async ({ page }) => {
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
    const y = box.y + box.height / 2;
    const leftX = box.x + box.width * 0.25;
    const rightX = box.x + box.width * 0.75;

    await page.mouse.move(leftX, y);
    await page.mouse.down();
    expect(await shown(), 'pressing the left half shows A').toBe('a');
    // The view switch tracks the hold — with no overlay on the picture, that
    // highlight is the observer's feedback about which variant they are seeing.
    await expect(page.locator('.view-switch button[data-view="a"]')).toHaveClass(/\bon\b/);
    await page.mouse.up();
    expect(await shown(), 'releasing returns to the original').toBe('ref');

    await page.mouse.move(rightX, y);
    await page.mouse.down();
    expect(await shown(), 'pressing the right half shows B').toBe('b');
    await expect(page.locator('.view-switch button[data-view="b"]')).toHaveClass(/\bon\b/);
    await page.mouse.up();
    expect(await shown()).toBe('ref');
  });

  // Panning has to keep working under a hold, and crossing the midline mid-drag
  // must NOT swap the variant — that would change the picture out from under a
  // comparison the observer is in the middle of making.
  test('the half is decided on press and survives a drag across the midline', async ({ page }) => {
    await toTrial(page);
    expect(await toKind(page, 'pair'), 'needed a pair trial').toBe(true);
    await page.locator('#input-mode').selectOption('hold');
    await page.waitForSelector('.trial[data-input-mode="hold"]');
    await page.waitForSelector('.viewport:not(.is-loading)');

    const shown = () =>
      page.evaluate(
        () => document.querySelector<HTMLImageElement>('.viewport img.shown')!.dataset.layer,
      );
    const box = (await page.locator('#viewport').boundingBox())!;
    const y = box.y + box.height / 2;

    await page.mouse.move(box.x + box.width * 0.2, y);
    await page.mouse.down();
    expect(await shown()).toBe('a');
    // Drag well past the centre into the right half.
    for (let i = 1; i <= 6; i++) {
      await page.mouse.move(box.x + box.width * (0.2 + i * 0.1), y);
    }
    expect(await shown(), 'crossing the midline must not swap A for B mid-gesture').toBe('a');
    await page.mouse.up();
    expect(await shown()).toBe('ref');
  });

  // A single-stimulus trial has no B, so the halves collapse to one gesture:
  // hold to see the encoding, release for the original.
  test('on a single-stimulus trial either half shows the compressed image', async ({ page }) => {
    await toTrial(page);
    expect(await toKind(page, 'single'), 'needed a rating trial').toBe(true);
    await page.locator('#input-mode').selectOption('hold');
    await page.waitForSelector('.trial[data-input-mode="hold"]');
    await page.waitForSelector('.viewport:not(.is-loading)');

    const shown = () =>
      page.evaluate(
        () => document.querySelector<HTMLImageElement>('.viewport img.shown')!.dataset.layer,
      );
    expect(await shown()).toBe('ref');

    const box = (await page.locator('#viewport').boundingBox())!;
    const y = box.y + box.height / 2;
    for (const frac of [0.25, 0.75]) {
      await page.mouse.move(box.x + box.width * frac, y);
      await page.mouse.down();
      expect(await shown(), `half at ${frac} shows the compressed image`).toBe('a');
      await page.mouse.up();
      expect(await shown()).toBe('ref');
    }
  });
});
