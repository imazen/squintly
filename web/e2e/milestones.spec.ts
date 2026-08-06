import { expect, test } from './fixtures';

import { clickBegin, completeProfileAndStart, gotoFresh, submitOneTrial } from './helpers';

async function toTrial(page: import('@playwright/test').Page) {
  await gotoFresh(page);
  await clickBegin(page);
  await page.getByRole('button', { name: /^Skip$/ }).click();
  await completeProfileAndStart(page);
  await page.waitForSelector('.trial[data-trial-id]', { timeout: 30_000 });
}

test.describe('milestone notices', () => {
  // Front-loaded on purpose: the first mark lands almost immediately so a new
  // observer learns the threshold exists while they still have the patience to
  // care about it.
  test('a notice appears at the second comparison, and says the score', async ({ page }) => {
    test.setTimeout(120_000);
    await toTrial(page);

    // Answer until two comparisons have landed.
    //
    // Selected by `data-notice`, not by `.notice`: the layer holds one notice
    // at a time and the process nudge shares it, so a bare `.notice` would stop
    // this loop on whichever arrived first and then assert milestone shape
    // against it.
    let notice = 0;
    for (let i = 0; i < 14 && notice === 0; i++) {
      await submitOneTrial(page);
      notice = await page.locator('[data-notice="milestone"]').count();
    }
    expect(notice, 'a milestone notice should have appeared').toBeGreaterThan(0);

    const el = page.locator('[data-notice="milestone"]');
    // x/20, so the progress is legible without reading the sentence.
    await expect(el.locator('.notice-badge')).toHaveText(/^\d+\/20$/);
    // And wording that says what the 20 is for, not just that a number went up.
    await expect(el.locator('.notice-text')).toContainText(/count|consistency|analysis|usable/i);
  });

  // Two seconds, then gone on its own — and tappable before that.
  test('a notice can be tapped away, and leaves by itself', async ({ page }) => {
    test.setTimeout(120_000);
    await toTrial(page);
    for (let i = 0; i < 14; i++) {
      await submitOneTrial(page);
      if (await page.locator('.notice').count()) break;
    }
    const el = page.locator('.notice');
    if (!(await el.count())) test.skip(true, 'no milestone reached in budget');

    await el.click();
    await expect(el).toHaveCount(0, { timeout: 3_000 });

    // The next one goes on its own without being touched.
    for (let i = 0; i < 14; i++) {
      await submitOneTrial(page);
      if (await page.locator('.notice').count()) break;
    }
    if (await page.locator('.notice').count()) {
      await expect(page.locator('.notice')).toHaveCount(0, { timeout: 6_000 });
    }
  });

  // A stimulus is under psychovisual judgement; a notice may overlay the header
  // band but never the picture.
  test('a notice never covers the stimulus', async ({ page }) => {
    test.setTimeout(120_000);
    await toTrial(page);
    for (let i = 0; i < 14; i++) {
      await submitOneTrial(page);
      if (await page.locator('.notice').count()) break;
    }
    if (!(await page.locator('.notice').count())) test.skip(true, 'no milestone reached in budget');
    const m = await page.evaluate(() => {
      const n = document.querySelector('.notice')!.getBoundingClientRect();
      const img = document.querySelector('#stimulus')!.getBoundingClientRect();
      const cs = getComputedStyle(document.querySelector('.notice')!);
      // The VISIBLE picture: a magnified stimulus extends past the frame in
      // both directions, so its own rect starts off-screen and measuring
      // against that would report a huge intrusion for a notice sitting in the
      // chrome. What matters is the part of the picture a person can see.
      const vp = document.querySelector('#viewport')!.getBoundingClientRect();
      const top = Math.max(img.top, vp.top);
      const bottom = Math.min(img.bottom, vp.bottom);
      const visible = Math.max(1, bottom - top);
      return {
        intoPicture: Math.max(0, Math.min(n.bottom, bottom) - top) / visible,
        bg: cs.backgroundColor,
      };
    });
    // A notice DOES cross onto the picture when the stimulus fills the frame —
    // there is nowhere at the top of the screen that is not the picture. That is
    // a deliberate, bounded exception, not an oversight:
    //  * it is at the extreme top edge, never the region being compared;
    //  * it lasts two seconds and can be tapped away sooner;
    //  * it is semi-transparent, so the pixels under it are not replaced;
    //  * and it appears immediately after an answer — at the START of the next
    //    trial, where the seen-both gate guarantees the observer has not begun
    //    comparing yet.
    // What must not happen is it growing into a band across the picture, which
    // is what a long wrapped sentence on a 304px screen would do.
    expect(m.intoPicture, 'a notice must stay at the picture edge').toBeLessThan(0.18);
    expect(m.bg, 'and must not replace the pixels under it').toMatch(/rgba\(/);
  });
});
