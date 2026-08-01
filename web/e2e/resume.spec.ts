import { expect, test, type Page } from '@playwright/test';

import { clickBegin, completeProfileAndStart, gotoFresh } from './helpers';

/// Walk a fresh visitor all the way through onboarding into trials.
async function onboard(page: Page) {
  await gotoFresh(page);
  await clickBegin(page);
  await page.getByRole('button', { name: /^Skip$/ }).click();
  await completeProfileAndStart(page);
  await page.waitForSelector('.trial[data-trial-id]', { timeout: 30_000 });
}

test.describe('reopening the app', () => {
  // Reopening used to drop a returning observer back on the welcome screen and
  // walk them through Begin -> calibration -> profile again, even though the
  // observer id, profile and calibration were all already stored — three
  // screens of friction in front of the thing they came back to do.
  //
  // The property is "no interaction required", not "a particular screen is
  // visible": resume is fast enough that the interstitial can come and go
  // between polls, so asserting on it would be timing-dependent.
  test('a returning observer lands in trials with no interaction', async ({ page }) => {
    await onboard(page);

    await page.goto('/');
    // Deliberately no clicks.
    await page.waitForSelector('.trial[data-trial-id]', { timeout: 30_000 });
    await expect(page.locator('.rating-panel, .pair-panel')).toBeVisible();
    await expect(
      page.getByRole('heading', { name: /Image Discrimination Study/i }),
    ).toHaveCount(0);
  });

  test('a first-time visitor still sees the welcome screen', async ({ page }) => {
    await gotoFresh(page);
    await expect(page.getByRole('heading', { name: /Image Discrimination Study/i })).toBeVisible();
    await expect(page.locator('.trial[data-trial-id]')).toHaveCount(0);
  });

  // Resuming must not be a trap: there has to be a way back. Hold the session
  // request so the interstitial stays up long enough to click, rather than
  // racing it.
  test('a returning observer can decline and start from the beginning', async ({ page }) => {
    await onboard(page);

    let release: () => void = () => {};
    const gate = new Promise<void>((r) => (release = r));
    await page.route('**/api/session', async (route) => {
      await gate;
      await route.continue();
    });

    await page.goto('/');
    await page.locator('#not-now').click();
    await expect(page.getByRole('heading', { name: /Image Discrimination Study/i })).toBeVisible();
    release();
  });
});

test.describe('calibration stickiness', () => {
  // The card measurement was persisted, but the screen reopened at a fixed
  // slider value and its Skip returned nulls that the caller wrote straight
  // over the stored value — so re-entering just to check could destroy it.
  test('a stored measurement is reloaded, and Skip preserves it', async ({ page }) => {
    await gotoFresh(page);
    await clickBegin(page);

    const slider = page.locator('#slider');
    await slider.fill('320');
    await slider.dispatchEvent('input');
    await page.getByRole('button', { name: /Looks right/i }).click();
    // Stage 2 is the distance step; skip it — stage 1's value must survive.
    await page.getByRole('button', { name: /^Skip$/ }).click();

    const stored = () =>
      page.evaluate(() => JSON.parse(localStorage.getItem('squintly:calibration') || 'null'));
    await expect.poll(async () => (await stored())?.css_px_per_mm ?? 0).toBeGreaterThan(0);
    const first = await stored();

    // Back to the welcome screen (this observer has no profile yet, so a plain
    // reload lands there) and reopen calibration from the link.
    await page.goto('/');
    await page.locator('#calibrate-link').click();
    await expect(page.locator('#slider')).toBeVisible();

    // It must reopen where it was left, not at a default.
    const value = Number(await page.locator('#slider').inputValue());
    expect(Math.abs(value - 320), `slider reopened at ${value}, expected ~320`).toBeLessThan(2);

    await page.getByRole('button', { name: /^Skip$/ }).click();
    const after = await stored();
    expect(after?.css_px_per_mm, 'Skip must preserve, not erase').toBeCloseTo(
      first.css_px_per_mm,
      3,
    );
  });

  // Calibrate was a permanent tab; it is a one-off measurement the app
  // remembers, so it lives behind a link that says whether it is already done.
  test('calibration is a link, not a tab', async ({ page }) => {
    await gotoFresh(page);
    await expect(page.locator('.squintly-tabs button[data-tab="calibrate"]')).toHaveCount(0);
    await expect(page.locator('#calibrate-link')).toContainText(/calibrate screen size/i);
  });
});
