import { expect, test } from '@playwright/test';

import { gotoFresh } from './helpers';

test.describe('welcome screen', () => {
  test('renders research-impact framing copy', async ({ page }) => {
    await gotoFresh(page);
    await expect(page.getByRole('heading', { name: /Image Discrimination Study/ })).toBeVisible();
    await expect(page.getByText(/perceptual quality metric/i)).toBeVisible();
    await expect(page.getByText(/No login, no personal info/i)).toBeVisible();
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
