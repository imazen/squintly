import { expect, test } from '@playwright/test';

import { clickBegin, gotoFresh } from './helpers';

test.describe('calibration', () => {
  test('skip path routes to profile form without recording mm', async ({ page }) => {
    await gotoFresh(page);
    await clickBegin(page);
    await page.getByRole('button', { name: /^Skip$/ }).click();
    await expect(page.getByRole('heading', { name: /A few quick questions/i })).toBeVisible();
    const calib = await page.evaluate(() => localStorage.getItem('squintly:calibration'));
    expect(calib).not.toBeNull();
    const parsed = JSON.parse(calib!);
    expect(parsed.css_px_per_mm).toBeNull();
    expect(parsed.viewing_distance_cm).toBeNull();
  });

  test('card-resize then skip blind-spot persists css_px_per_mm', async ({ page }) => {
    await gotoFresh(page);
    await clickBegin(page);
    // Move the slider to 300, then accept. With a non-null pxPerMm we route to
    // the blind-spot test (not the distance preset).
    const slider = page.locator('#slider');
    await slider.evaluate((el: HTMLInputElement) => {
      el.value = '300';
      el.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await page.getByRole('button', { name: /Looks right/i }).click();
    await expect(page.getByRole('heading', { name: /Blind-spot test/i })).toBeVisible();
    // Skip the blind-spot — we keep the calibrated mm value, leave the distance null.
    await page.locator('#skip2').click();
    await expect(page.getByRole('heading', { name: /A few quick questions/i })).toBeVisible();
    const calib = JSON.parse(
      (await page.evaluate(() => localStorage.getItem('squintly:calibration')))!,
    );
    // 300 CSS px / 85.6 mm ≈ 3.50 px/mm
    expect(calib.css_px_per_mm).toBeGreaterThan(3.4);
    expect(calib.css_px_per_mm).toBeLessThan(3.6);
    expect(calib.viewing_distance_cm).toBeNull();
  });
});
