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
