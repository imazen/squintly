import { expect, test } from './fixtures';

import { gotoFresh } from './helpers';

test.describe('welcome screen', () => {
  test('renders the make-the-web-faster framing copy', async ({ page }) => {
    await gotoFresh(page);
    await expect(page.getByRole('heading', { name: /Image Discrimination Study/ })).toBeVisible();
    // Scoped to the screen's own paragraphs: the study picker's summary for the
    // main study paraphrases this copy, so an unscoped getByText matches twice
    // and fails on strict mode rather than on the copy being absent.
    const intro = page.locator('[data-screen="welcome"] > p');
    await expect(intro.filter({ hasText: /make the web faster/i })).toBeVisible();
    await expect(intro.filter({ hasText: /perceptual quality metric/i })).toBeVisible();
    await expect(intro.filter({ hasText: /No login required/i })).toBeVisible();
    await expect(page.getByRole('button', { name: /^Begin$/ })).toBeEnabled();
  });

  test('shows JXL flag hint to Chromium observers without native JXL', async ({ page }) => {
    await gotoFresh(page);
    // Playwright Chromium ships without JXL by default; we expect the hint.
    await expect(page.getByText(/chrome:\/\/flags\/#enable-jxl-image-format/)).toBeVisible();
  });

  test('begin advances to calibration', async ({ page }) => {
    await gotoFresh(page);
    await page.getByRole('button', { name: /^Begin$/ }).click();
    await expect(page.getByRole('heading', { name: /Calibration/i })).toBeVisible();
  });
});
