// End-to-end test for the public suggestion form.
//
// Drives the UI through file pick → fill required fields → submit →
// confirms the success card. Then validates the row landed via the
// (admin-token-protected) listing endpoint.
//
// The Squintly binary in the e2e harness boots without
// SQUINTLY_SUGGESTION_ADMIN_TOKEN configured, so the listing endpoint
// returns 503 — we assert that posture instead of trying to read the
// admin queue. The submit endpoint is open to the public so the form
// itself is the integration point.

import { expect, test } from '@playwright/test';

test.describe('public suggestion form', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
    await page.evaluate(() => localStorage.clear());
  });

  test('welcome tab bar exposes Suggest', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('.squintly-tabs button[data-tab="suggest"]')).toBeVisible();
  });

  test('submit a tiny PNG with required fields', async ({ page, request }) => {
    await page.goto('/');
    await page.locator('.squintly-tabs button[data-tab="suggest"]').click();
    await expect(page.locator('[data-screen="suggest"]')).toBeVisible();

    // Drop a 1x1 PNG into the file input.
    const onePxPng = Buffer.from(
      'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=',
      'base64',
    );
    await page.locator('#file').setInputFiles({
      name: 'tiny.png',
      mimeType: 'image/png',
      buffer: onePxPng,
    });
    await expect(page.locator('#preview')).toBeVisible();

    await page.locator('#email').fill('e2e-tester@example.com');
    await page.locator('#page').fill('https://example.com/test-photo');
    await page.locator('#license').selectOption('cc-by');
    await page.locator('#license-text').fill('Test attribution: e2e harness.');
    await page.locator('#why').fill('Smoke-test of the suggest UI.');

    await Promise.all([
      page.waitForResponse((r) => r.url().includes('/api/suggestions') && r.request().method() === 'POST'),
      page.locator('#submit').click(),
    ]);
    await expect(page.locator('#suggest-result')).toBeVisible();
    await expect(page.locator('#suggest-result h2')).toContainText(/submission #\d+ received/);

    // Without an admin token configured, the list endpoint must refuse access:
    // 503 (env unset) or 400 (token mismatch). Either is acceptable; what we're
    // asserting is the public can't list other people's submissions.
    const listing = await request.get('/api/suggestions?status=pending');
    expect([400, 503]).toContain(listing.status());
  });

  test('rejects submission without email', async ({ page }) => {
    await page.goto('/');
    await page.locator('.squintly-tabs button[data-tab="suggest"]').click();
    const onePxPng = Buffer.from(
      'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=',
      'base64',
    );
    await page.locator('#file').setInputFiles({
      name: 'tiny.png',
      mimeType: 'image/png',
      buffer: onePxPng,
    });
    await page.locator('#page').fill('https://example.com/x');
    // Leave email empty. HTML-level required attribute should block the submit.
    const emailInvalid = await page.locator('#email').evaluate((el: HTMLInputElement) => {
      el.checkValidity();
      return el.validationMessage;
    });
    expect(emailInvalid.length).toBeGreaterThan(0);
  });
});
