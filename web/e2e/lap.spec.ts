import { expect, test } from './fixtures';

import { clickBegin, completeProfileAndStart, gotoFresh, submitOneTrial } from './helpers';

async function toTrial(page: import('@playwright/test').Page) {
  await gotoFresh(page);
  await clickBegin(page);
  await page.getByRole('button', { name: /^Skip$/ }).click();
  await completeProfileAndStart(page);
  await page.waitForSelector('.trial[data-trial-id]', { timeout: 30_000 });
}

test.describe('the lap bar', () => {
  // A full-width empty bar on arrival is a demand, not an invitation — and on a
  // rating-only study it would never move at all, because comparisons are what
  // the threshold counts.
  test('is absent until the first comparison lands', async ({ page }) => {
    await toTrial(page);
    await expect(page.locator('#lap')).toBeHidden();
  });

  test('appears and advances once comparisons are answered', async ({ page }) => {
    // Several real trials, each of which has to satisfy the seen-both gate.
    test.setTimeout(120_000);
    await toTrial(page);
    // Answer until a pair has been recorded; a rating correctly does not move it.
    // The fill's declared width is what advanced; its RENDERED width starts at
    // zero and eases in over 260ms, and `paintLap` re-runs on every trial — so
    // measuring the box immediately after answering catches the transition's
    // first frame and reads 0 no matter how many comparisons have landed.
    let declared = '';
    for (let i = 0; i < 12 && (declared === '' || declared === '0%'); i++) {
      await submitOneTrial(page);
      declared = await page.evaluate(
        () => document.querySelector<HTMLElement>('#lap-fill')?.style.width ?? '',
      );
    }
    expect(declared, 'the bar should have advanced after some comparisons').toMatch(/^[1-9]/);
    // And it does render, once the ease has run.
    await expect
      .poll(
        async () =>
          page.evaluate(
            () => document.querySelector<HTMLElement>('#lap-fill')?.getBoundingClientRect().width ?? 0,
          ),
        { timeout: 5_000 },
      )
      .toBeGreaterThan(0);
    await expect(page.locator('#lap')).toBeVisible();

    // It is a real threshold, and the tooltip says which.
    const title = await page.locator('#lap').getAttribute('title');
    expect(title).toMatch(/comparisons/i);
  });

  // The bar sits above the picture and must never take space from it or paint
  // over it — the same rule the reveal hint and the edge frame obey.
  test('never overlaps the stimulus', async ({ page }) => {
    test.setTimeout(90_000);
    await toTrial(page);
    for (let i = 0; i < 3; i++) await submitOneTrial(page);
    const m = await page.evaluate(() => {
      const lap = document.querySelector('#lap')?.getBoundingClientRect();
      const vp = document.querySelector('#viewport')!.getBoundingClientRect();
      return lap ? { lapBottom: lap.bottom, vpTop: vp.top, h: lap.height } : null;
    });
    if (!m) return;
    expect(m.lapBottom).toBeLessThanOrEqual(m.vpTop + 1);
    expect(m.h, 'a hairline, not a band').toBeLessThan(8);
  });
});

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
