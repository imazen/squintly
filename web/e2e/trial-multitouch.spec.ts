import { expect, test, type Page } from '@playwright/test';

import {
  clickBegin,
  completeProfileAndStart,
  gotoFresh,
  satisfyGate,
  useMode,
} from './helpers';

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
  await satisfyGate(page);
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
    await useMode(page, 'hold');
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

    // Reset to 1x so there is headroom to magnify into. An undersized stimulus
    // is magnified to cover the frame on load, and a small source can land on
    // the ladder's 8x cap — where spreading the fingers correctly does nothing.
    // `0` is an explicit choice, which outranks the cover default.
    await page.keyboard.press('0');
    await expect(page.locator('#zoom-readout')).toHaveText('1×');
    const start = 1;

    // Two fingers 60px apart, spread to 240px — a 4x span.
    await touch(page, 'pointerdown', 1, cx - 30, cy);
    await touch(page, 'pointerdown', 2, cx + 30, cy);
    for (const half of [60, 90, 120]) {
      await touch(page, 'pointermove', 1, cx - half, cy);
      await touch(page, 'pointermove', 2, cx + half, cy);
    }
    const zoomed = Number((await page.locator('#zoom-readout').textContent())!.replace('×', ''));
    expect(zoomed, 'spreading the fingers must magnify').toBeGreaterThan(start);
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

    // What the fit *should* be for this image in this frame. Computed rather
    // than assumed: the sampler is random, and asserting merely that the zoom
    // "changed" is wrong whenever the current factor already is the fit.
    const expectedFit = await page.evaluate(() => {
      const i = document.querySelector<HTMLImageElement>('#stimulus')!;
      const r = document.querySelector<HTMLElement>('#viewport')!.getBoundingClientRect();
      let best = 1;
      for (const z of [1, 2, 3, 4, 5, 6, 7, 8]) {
        if ((i.naturalWidth * z) / devicePixelRatio <= r.width &&
            (i.naturalHeight * z) / devicePixelRatio <= r.height) best = z;
      }
      return best;
    });

    // Magnify away from wherever we are, then double-tap back.
    await page.keyboard.press('ArrowUp');
    await page.keyboard.press('ArrowUp');

    await touch(page, 'pointerdown', 1, cx, cy);
    await touch(page, 'pointerup', 1, cx, cy);
    await touch(page, 'pointerdown', 1, cx, cy);
    await touch(page, 'pointerup', 1, cx, cy);

    await expect(page.locator('#zoom-readout')).toHaveText(`${expectedFit}×`);

    const m = await page.evaluate(() => {
      const i = document.querySelector<HTMLImageElement>('#stimulus')!;
      const vp = document.querySelector<HTMLElement>('#viewport')!;
      const ir = i.getBoundingClientRect();
      const vr = vp.getBoundingClientRect();
      return {
        factor: (ir.width * window.devicePixelRatio) / i.naturalWidth,
        fitsW: ir.width <= vr.width + 1,
        fitsH: ir.height <= vr.height + 1,
        // Centred means symmetric overhang, not a particular transform string —
        // the transform now carries a -50% centring term as well as the pan.
        leftGap: ir.left - vr.left,
        rightGap: vr.right - ir.right,
      };
    });
    expect(m.factor, 'never below 1:1').toBeGreaterThanOrEqual(0.98);
    expect(Math.abs(m.factor - Math.round(m.factor)), 'whole factor only').toBeLessThan(0.02);
    // At the fit factor the image is inside the frame — unless it is an
    // oversized source, which cannot fit and correctly lands on 1x.
    if (m.factor > 1) {
      expect(m.fitsW && m.fitsH, 'at >1x the whole image must fit').toBe(true);
    }
    expect(
      Math.abs(m.leftGap - m.rightGap),
      `fitting re-centres (left ${m.leftGap}, right ${m.rightGap})`,
    ).toBeLessThan(2);
  });
});

test.describe('panning reaches the edges', () => {
  // `inset: 0; margin: auto` only centres a box SMALLER than its container.
  // Once the image overflowed, CSS resolved the over-constraint by honouring
  // `left` and dumping the excess into `margin-right`, so the image sat flush
  // left while the pan limits still assumed a centred crop (+/- half the
  // overflow). Horizontal panning therefore reached only halfway to the right
  // edge — worst on square and landscape sources, where the overflow is
  // horizontal. The vertical axis was unaffected, because that over-constraint
  // IS split evenly, which is why the bug looked orientation-dependent.
  test('an oversized stimulus is centred on both axes', async ({ page }) => {
    await toTrial(page);

    // Find an oversized trial.
    let found = false;
    for (let i = 0; i < 60 && !found; i++) {
      await page.waitForSelector('.viewport.all-ready', { timeout: 15_000 }).catch(() => {});
      found = await page.evaluate(() => {
        const im = document.querySelector<HTMLImageElement>('#stimulus');
        const vp = document.querySelector<HTMLElement>('#viewport');
        if (!im || !vp) return false;
        const r = im.getBoundingClientRect();
        const v = vp.getBoundingClientRect();
        return r.width > v.width + 20 && r.height > v.height + 20;
      });
      if (!found) await advance(page);
    }
    expect(found, 'no stimulus overflowed on both axes').toBe(true);

    const gaps = await page.evaluate(() => {
      const r = document.querySelector<HTMLImageElement>('#stimulus')!.getBoundingClientRect();
      const v = document.querySelector<HTMLElement>('#viewport')!.getBoundingClientRect();
      return {
        left: r.left - v.left,
        right: v.right - r.right,
        top: r.top - v.top,
        bottom: v.bottom - r.bottom,
      };
    });
    // Symmetric overhang on each axis == centred. Asymmetry is the bug.
    expect(Math.abs(gaps.left - gaps.right), `left ${gaps.left} vs right ${gaps.right}`).toBeLessThan(2);
    expect(Math.abs(gaps.top - gaps.bottom), `top ${gaps.top} vs bottom ${gaps.bottom}`).toBeLessThan(2);
  });

  test('dragging can reach every edge of an oversized stimulus', async ({ page }) => {
    await toTrial(page);

    let found = false;
    for (let i = 0; i < 60 && !found; i++) {
      await page.waitForSelector('.viewport.all-ready', { timeout: 15_000 }).catch(() => {});
      found = await page.evaluate(() => {
        const im = document.querySelector<HTMLImageElement>('#stimulus');
        const vp = document.querySelector<HTMLElement>('#viewport');
        if (!im || !vp) return false;
        const r = im.getBoundingClientRect();
        const v = vp.getBoundingClientRect();
        return r.width > v.width + 20 && r.height > v.height + 20;
      });
      if (!found) await advance(page);
    }
    expect(found, 'no stimulus overflowed on both axes').toBe(true);

    const box = (await page.locator('#viewport').boundingBox())!;
    const cx = box.x + box.width / 2;
    const cy = box.y + box.height / 2;

    /// Drag far enough to hit the clamp in the given direction, then report
    /// which frame edge the image edge has been brought to.
    const dragTo = async (dx: number, dy: number) => {
      await touch(page, 'pointerdown', 1, cx, cy);
      // Well beyond any plausible limit, in steps so the drag registers.
      for (let s = 1; s <= 10; s++) {
        await touch(page, 'pointermove', 1, cx + (dx * s) / 10, cy + (dy * s) / 10);
      }
      await touch(page, 'pointerup', 1, cx + dx, cy + dy);
      return page.evaluate(() => {
        const r = document.querySelector<HTMLImageElement>('#stimulus')!.getBoundingClientRect();
        const v = document.querySelector<HTMLElement>('#viewport')!.getBoundingClientRect();
        return {
          left: r.left - v.left,
          right: v.right - r.right,
          top: r.top - v.top,
          bottom: v.bottom - r.bottom,
        };
      });
    };

    // Drag right → the image's LEFT edge comes into the frame (gap → 0).
    const atLeft = await dragTo(4000, 0);
    expect(Math.abs(atLeft.left), `left edge unreachable, gap ${atLeft.left}`).toBeLessThan(2);

    // Drag left → the RIGHT edge. This is the one that was unreachable.
    const atRight = await dragTo(-8000, 0);
    expect(Math.abs(atRight.right), `right edge unreachable, gap ${atRight.right}`).toBeLessThan(2);

    // And both vertical edges.
    const atTop = await dragTo(0, 4000);
    expect(Math.abs(atTop.top), `top edge unreachable, gap ${atTop.top}`).toBeLessThan(2);
    const atBottom = await dragTo(0, -8000);
    expect(Math.abs(atBottom.bottom), `bottom edge unreachable, gap ${atBottom.bottom}`).toBeLessThan(2);
  });
});

test.describe('undersized stimuli', () => {
  // An S-bucket source is 240px, which at 1:1 on a DPR-3 phone is ~80 CSS px —
  // a postage stamp with acres of black around it and no way to see the
  // artefacts being rated. Magnifying to cover is the only fix available: below
  // 1:1 resamples the encode, above it at integer nearest-neighbour invents
  // nothing.
  test('a stimulus smaller than the frame is magnified to cover it', async ({ page }) => {
    await toTrial(page);

    let checked = 0;
    for (let i = 0; i < 40 && checked < 3; i++) {
      await page.waitForSelector('.viewport.all-ready', { timeout: 15_000 }).catch(() => {});
      const m = await page.evaluate(() => {
        const im = document.querySelector<HTMLImageElement>('#stimulus');
        const vp = document.querySelector<HTMLElement>('#viewport');
        if (!im || !vp || !im.naturalWidth) return null;
        const r = im.getBoundingClientRect();
        const v = vp.getBoundingClientRect();
        // Could this source cover the frame at some whole factor <= 8?
        const maxW = (im.naturalWidth * 8) / devicePixelRatio;
        const maxH = (im.naturalHeight * 8) / devicePixelRatio;
        return {
          coverable: maxW >= v.width && maxH >= v.height,
          coversW: r.width >= v.width - 1,
          coversH: r.height >= v.height - 1,
          factor: (r.width * devicePixelRatio) / im.naturalWidth,
        };
      });
      if (m?.coverable) {
        checked += 1;
        expect(
          m.coversW && m.coversH,
          `stimulus does not cover the frame (factor ${m.factor})`,
        ).toBe(true);
        // Still a whole factor, still never a downscale.
        expect(Math.abs(m.factor - Math.round(m.factor))).toBeLessThan(0.02);
        expect(m.factor).toBeGreaterThanOrEqual(0.98);
      }
      await advance(page);
    }
    expect(checked, 'expected some coverable stimuli to check').toBeGreaterThan(0);
  });

  // Magnification persists across trials on purpose, so covering must only ever
  // raise it — a deliberate 8x must survive a small source.
  test('covering never reduces a magnification the observer chose', async ({ page }) => {
    await toTrial(page);
    await page.keyboard.press('0');
    for (let i = 0; i < 7; i++) await page.keyboard.press('ArrowUp'); // to 8x
    await expect(page.locator('#zoom-readout')).toHaveText('8×');

    for (let i = 0; i < 4; i++) {
      await advance(page);
      const z = Number((await page.locator('#zoom-readout').textContent())!.replace('×', ''));
      expect(z, 'covering must not lower a chosen magnification').toBeGreaterThanOrEqual(8);
    }
  });
});
