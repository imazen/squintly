import { expect, test } from './fixtures';

import {
  clickBegin,
  completeProfileAndStart,
  gotoLanding,
  submitOneTrial,
} from './helpers';

test.describe('the front page', () => {
  // Opening squintly used to run straight into onboarding, and for a returning
  // observer straight into the next trial — so the decision to take part was
  // made on the one screen with nothing on it to decide from.
  test('explains the task and offers a way in without starting one', async ({ page }) => {
    await gotoLanding(page);
    await expect(page.locator('[data-screen="landing"]')).toBeVisible();
    await expect(page.locator('.trial[data-trial-id]')).toHaveCount(0);

    // What the judgement actually is, not just that there is one.
    await expect(page.locator('.lede')).toContainText(/closer to the original/i);
    await expect(page.locator('#landing-start')).toBeVisible();
  });

  // Both doors, and neither dressed as the lesser one: an anonymous observer's
  // data is worth exactly as much as a signed-in one's.
  test('offers guest and sign-in, and says what signing in buys', async ({ page }) => {
    await gotoLanding(page);
    await expect(page.locator('#landing-start')).toContainText(/guest/i);
    await expect(page.locator('#landing-signin')).toBeVisible();
    await expect(page.locator('[data-screen="landing"]')).toContainText(
      /carry the same reviewer identity/i,
    );
  });

  // A raw response count says nothing without the target beside it — 400
  // ratings is most of the way for one study and a rounding error for another.
  test('shows each study against its own targets', async ({ page }) => {
    await gotoLanding(page);
    const goals = page.locator('.goal');
    expect(await goals.count()).toBeGreaterThan(0);
    const first = goals.first();
    await expect(first.locator('.goal-name')).not.toBeEmpty();
    await expect(first.locator('.goal-bar')).toBeVisible();
    // The legend names a threshold, so the bar cannot be an unlabelled sliver.
    await expect(first.locator('.goal-legend')).toContainText(/\d/);
  });

  // A board you cannot find yourself on is a scoreboard for other people.
  test('marks your own row once you have rated', async ({ page }) => {
    await gotoLanding(page);
    await page.locator('#landing-start').click();
    await clickBegin(page);
    await page.getByRole('button', { name: /^Skip$/ }).click();
    await completeProfileAndStart(page);
    await submitOneTrial(page);

    await page.goto('/');
    await expect(page.locator('[data-screen="landing"]')).toBeVisible();
    const me = page.locator('.board tr.me');
    await expect(me).toHaveCount(1);
    await expect(me.locator('.you')).toBeVisible();
    // And it is pulled into the shown slice rather than left below the fold.
    await expect(page.locator('.board tbody tr').first()).toHaveClass(/me/);
  });

  test('the front page is where a session ends up, not the next trial', async ({ page }) => {
    await gotoLanding(page);
    await page.locator('#landing-start').click();
    await clickBegin(page);
    await page.getByRole('button', { name: /^Skip$/ }).click();
    await completeProfileAndStart(page);
    await page.waitForSelector('.trial[data-trial-id]', { timeout: 30_000 });

    await page.goto('/');
    await expect(page.locator('[data-screen="landing"]')).toBeVisible();
  });
});
