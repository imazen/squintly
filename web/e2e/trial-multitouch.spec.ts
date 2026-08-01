import { expect, test, type Page } from '@playwright/test';

import { clickBegin, completeProfileAndStart, gotoFresh } from './helpers';

async function toTrial(page: Page) {
  await gotoFresh(page);
  await clickBegin(page);
  await page.getByRole('button', { name: /^Skip$/ }).click();
  await completeProfileAndStart(page);
  await page.waitForSelector('.trial[data-trial-id]');
  await page.waitForSelector('.viewport:not(.is-loading)');
}

async function advance(page: Page) {
  const before = await page.locator('.trial').getAttribute('data-trial-id');
  await page.locator('.rating-panel button, .pair-panel button').first().click();
  await expect
    .poll(async () => page.locator('.trial').getAttribute('data-trial-id'), { timeout: 15_000 })
    .not.toBe(before);
  await page.waitForSelector('.viewport:not(.is-loading)');
}

async function toKind(page: Page, kind: 'single' | 'pair') {
  const sel = kind === 'pair' ? '.pair-panel' : '.rating-panel';
  for (let i = 0; i < 60; i++) {
    if (await page.locator(sel).count()) return true;
    await advance(page);
  }
  return false;
}

const shown = (page: Page) =>
  page.evaluate(
    () => document.querySelector<HTMLImageElement>('.viewport img.shown')!.dataset.layer,
  );

/// Drive raw pointer events so multiple touches can be alive at once —
/// `page.mouse` only ever models one.
async function touch(
  page: Page,
  type: 'pointerdown' | 'pointermove' | 'pointerup',
  id: number,
  x: number,
  y: number,
) {
  await page.evaluate(
    ({ type, id, x, y }) => {
      const vp = document.querySelector('#viewport')!;
      vp.dispatchEvent(
        new PointerEvent(type, {
          pointerId: id,
          pointerType: 'touch',
          isPrimary: id === 1,
          clientX: x,
          clientY: y,
          bubbles: true,
          cancelable: true,
          button: 0,
          buttons: type === 'pointerup' ? 0 : 1,
        }),
      );
    },
    { type, id, x, y },
  );
}

test.describe('multi-touch', () => {
  // Reported on a phone: one finger on the left, a second on the right, then
  // lift the first — the original appeared even though a finger was still
  // down. The single-pointer model ran the end-of-gesture handler on the first
  // release and then ignored the second finger entirely, because its id no
  // longer matched.
  test('lifting one finger while another is held keeps that finger showing', async ({ page }) => {
    await toTrial(page);
    expect(await toKind(page, 'pair'), 'needed a pair trial').toBe(true);
    await page.locator('#input-mode').selectOption('hold');
    await page.waitForSelector('.trial[data-input-mode="hold"]');
    await page.waitForSelector('.viewport:not(.is-loading)');

    const box = (await page.locator('#viewport').boundingBox())!;
    const y = box.y + box.height / 2;
    const leftX = box.x + box.width * 0.25;
    const rightX = box.x + box.width * 0.75;

    expect(await shown(page), 'resting view').toBe('ref');
    await touch(page, 'pointerdown', 1, leftX, y);
    expect(await shown(page), 'first finger on the left shows A').toBe('a');

    await touch(page, 'pointerdown', 2, rightX, y);
    // Second finger down is a pinch, so the picture must not change under it.
    expect(await shown(page), 'a second finger must not swap the variant').toBe('a');

    await touch(page, 'pointerup', 1, leftX, y);
    expect(
      await shown(page),
      'a finger is still down on the right — the original must not appear',
    ).toBe('b');

    await touch(page, 'pointerup', 2, rightX, y);
    expect(await shown(page), 'all fingers up returns to the original').toBe('ref');
  });

  test('pinching magnifies, and only ever by whole factors', async ({ page }) => {
    await toTrial(page);
    const box = (await page.locator('#viewport').boundingBox())!;
    const cy = box.y + box.height / 2;
    const cx = box.x + box.width / 2;

    await expect(page.locator('#zoom-readout')).toHaveText('1×');

    // Two fingers 60px apart, spread to 240px — a 4x span.
    await touch(page, 'pointerdown', 1, cx - 30, cy);
    await touch(page, 'pointerdown', 2, cx + 30, cy);
    for (const half of [60, 90, 120]) {
      await touch(page, 'pointermove', 1, cx - half, cy);
      await touch(page, 'pointermove', 2, cx + half, cy);
    }
    const zoomed = Number((await page.locator('#zoom-readout').textContent())!.replace('×', ''));
    expect(zoomed, 'spreading the fingers must magnify').toBeGreaterThan(1);
    expect(Number.isInteger(zoomed), `zoom ${zoomed} must be a whole factor`).toBe(true);

    // And the rendered factor really is that whole number, not a CSS scale.
    const factor = await page.evaluate(() => {
      const i = document.querySelector<HTMLImageElement>('#stimulus')!;
      return (i.getBoundingClientRect().width * window.devicePixelRatio) / i.naturalWidth;
    });
    expect(Math.abs(factor - Math.round(factor)), `rendered factor ${factor}`).toBeLessThan(0.02);

    // Pinching back in reduces it again.
    for (const half of [90, 60, 30]) {
      await touch(page, 'pointermove', 1, cx - half, cy);
      await touch(page, 'pointermove', 2, cx + half, cy);
    }
    const back = Number((await page.locator('#zoom-readout').textContent())!.replace('×', ''));
    expect(back).toBeLessThan(zoomed);

    await touch(page, 'pointerup', 1, cx - 30, cy);
    await touch(page, 'pointerup', 2, cx + 30, cy);
  });

  // Double tap resets magnification to "the whole image just fits". It can only
  // ever magnify a small stimulus up to the frame, never shrink a large one
  // down — below 1:1 the browser resamples the encode, which is the thing the
  // viewer exists to prevent. So an oversized source resolves to 1x.
  test('double tap fits the image at a whole factor and never goes below 1:1', async ({ page }) => {
    await toTrial(page);
    const box = (await page.locator('#viewport').boundingBox())!;
    const cx = box.x + box.width / 2;
    const cy = box.y + box.height / 2;

    // Magnify away from the fit, then double-tap back.
    await page.keyboard.press('ArrowUp');
    await page.keyboard.press('ArrowUp');
    const before = await page.locator('#zoom-readout').textContent();

    await touch(page, 'pointerdown', 1, cx, cy);
    await touch(page, 'pointerup', 1, cx, cy);
    await touch(page, 'pointerdown', 1, cx, cy);
    await touch(page, 'pointerup', 1, cx, cy);

    const after = (await page.locator('#zoom-readout').textContent())!;
    expect(after, `double tap should change magnification from ${before}`).not.toBe(before);

    const m = await page.evaluate(() => {
      const i = document.querySelector<HTMLImageElement>('#stimulus')!;
      const vp = document.querySelector<HTMLElement>('#viewport')!;
      const ir = i.getBoundingClientRect();
      const vr = vp.getBoundingClientRect();
      return {
        factor: (ir.width * window.devicePixelRatio) / i.naturalWidth,
        fitsW: ir.width <= vr.width + 1,
        fitsH: ir.height <= vr.height + 1,
        transform: i.style.transform,
      };
    });
    expect(m.factor, 'never below 1:1').toBeGreaterThanOrEqual(0.98);
    expect(Math.abs(m.factor - Math.round(m.factor)), 'whole factor only').toBeLessThan(0.02);
    // At the fit factor the image is inside the frame — unless it is an
    // oversized source, which cannot fit and correctly lands on 1x.
    if (m.factor > 1) {
      expect(m.fitsW && m.fitsH, 'at >1x the whole image must fit').toBe(true);
    }
    expect(m.transform, 'fitting re-centres').toBe('translate(0px, 0px)');
  });
});
