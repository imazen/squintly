import { expect, test } from './fixtures';

import {
  clickBegin,
  completeProfileAndStart,
  gotoFresh,
  signInAsAdmin,
  submitOneTrial,
} from './helpers';

test.describe('the pause menu', () => {
  async function openMenu(page: import('@playwright/test').Page) {
    await gotoFresh(page);
    await clickBegin(page);
    await page.getByRole('button', { name: /^Skip$/ }).click();
    await completeProfileAndStart(page);
    await page.waitForSelector('.trial[data-trial-id]', { timeout: 30_000 });
    await page.locator('#menu').click();
    await expect(page.getByRole('heading', { name: /^Pause$/ })).toBeVisible();
  }

  // A control that can be changed has to look like it. These inherited bare
  // button styling and read as flat text, so people did not know they opened.
  test('the dropdowns look like dropdowns', async ({ page }) => {
    await openMenu(page);
    for (const id of ['#menu-study', '#menu-mode']) {
      const sel = page.locator(id);
      if (!(await sel.count())) continue;
      const css = await sel.evaluate((el) => {
        const s = getComputedStyle(el);
        return { border: s.borderStyle, radius: s.borderRadius, image: s.backgroundImage };
      });
      expect(css.border, `${id} needs a visible border`).not.toBe('none');
      expect(parseFloat(css.radius), `${id} needs a rounded field`).toBeGreaterThan(0);
      // The chevron is drawn, so it cannot go missing with a font.
      expect(css.image, `${id} needs an affordance`).toContain('gradient');
    }
  });

  // Mid-session, looking at the board, wondering why you are not on it — that
  // is where somebody notices they are anonymous.
  test('offers sign-in while rating', async ({ page }) => {
    await openMenu(page);
    await expect(page.locator('#menu-account')).toContainText(/sign in/i);
  });

  // The operator view is for operators.
  test('admin is hidden from a participant', async ({ page }) => {
    await openMenu(page);
    await expect(page.locator('#menu-admin')).toBeHidden();
  });
});

test.describe('the end of a session', () => {
  // "You contributed N ratings. Close this tab" gave a volunteer no reason to
  // come back and no way to tell whether the N was any good.
  test('shows your stats and the board, not just a count', async ({ page }) => {
    await gotoFresh(page);
    await clickBegin(page);
    await page.getByRole('button', { name: /^Skip$/ }).click();
    await completeProfileAndStart(page);
    await submitOneTrial(page);

    await page.locator('#menu').click();
    await page.locator('#end').click();

    await expect(page.getByRole('heading', { name: /Thank you/i })).toBeVisible();
    await expect(page.locator('#done-board .board')).toBeVisible({ timeout: 15_000 });
    // Reliability sits beside volume, because volume alone is not the
    // contribution.
    await expect(page.locator('#done-board')).toContainText(/self-agree/i);
    // And a way back in, rather than "close this tab".
    await expect(page.locator('#done-again')).toBeVisible();
  });

  // The moment a guest has just seen their handle is when telling them it is
  // theirs to keep actually lands.
  test('a guest is invited to sign in, once, where it matters', async ({ page }) => {
    await gotoFresh(page);
    await clickBegin(page);
    await page.getByRole('button', { name: /^Skip$/ }).click();
    await completeProfileAndStart(page);
    await submitOneTrial(page);
    await page.locator('#menu').click();
    await page.locator('#end').click();

    await expect(page.locator('.signin-nudge')).toBeVisible({ timeout: 15_000 });
    await expect(page.locator('.signin-nudge')).toContainText(/across devices/i);
  });
});

test.describe('the admin view', () => {
  test('an admin reaches it from the menu and sees the study state', async ({
    page,
    coefficientPort,
  }) => {
    await gotoFresh(page);
    await signInAsAdmin(page, coefficientPort);
    await page.goto('/rate');
    await clickBegin(page);
    await page.getByRole('button', { name: /^Skip$/ }).click();
    await completeProfileAndStart(page);
    await page.waitForSelector('.trial[data-trial-id]', { timeout: 30_000 });

    await page.locator('#menu').click();
    const admin = page.locator('#menu-admin');
    await expect(admin).toBeVisible({ timeout: 15_000 });
    await admin.click();

    await expect(page.locator('[data-screen="admin"]')).toBeVisible();
    const body = page.locator('#admin-body');
    await expect(body).toContainText(/Responses/i, { timeout: 15_000 });
    // Each study against its own pre-registered targets.
    await expect(body).toContainText(/Min viable/i);
    // And reviewers with reliability beside volume.
    await expect(body).toContainText(/Self-agree/i);
    // Read-only: exclusion is a recorded disposition, not a button.
    await expect(page.locator('[data-screen="admin"]')).toContainText(/recorded disposition/i);

    await page.locator('#admin-back').click();
    await page.waitForSelector('.trial[data-trial-id]', { timeout: 30_000 });
  });
});
