import { expect, test } from '@playwright/test';

import { gotoFresh } from './helpers';

test.describe('optional email sign-in', () => {
  test('welcome screen exposes a sign-in link', async ({ page }) => {
    await gotoFresh(page);
    await expect(page.getByText(/Already have an email-linked account/i)).toBeVisible();
  });

  test('sign-in modal opens, validates email, and reports unconfigured Resend', async ({ page }) => {
    await gotoFresh(page);
    await page.getByText(/Already have an email-linked account/i).click();
    await expect(page.getByRole('heading', { name: /Save your progress/i })).toBeVisible();
    // Click "Send link" with no email — should warn.
    await page.getByRole('button', { name: /^Send link$/ }).click();
    await expect(page.locator('#signin-status')).toContainText(/email/i);
    // Provide an email; the test backend has no RESEND_API_KEY, so the start
    // endpoint returns 503 with a clear hint and the modal surfaces it.
    await page.locator('#signin-email').fill('observer@example.com');
    await page.getByRole('button', { name: /^Send link$/ }).click();
    await expect(page.locator('#signin-status')).toContainText(/not configured|Anonymous/i);
  });

  test('verify endpoint returns a friendly HTML page for an invalid token', async ({ request }) => {
    const r = await request.get('/api/auth/verify?token=' + 'a'.repeat(64));
    expect(r.ok()).toBeTruthy();
    const html = await r.text();
    expect(html).toContain('<!doctype html>');
    expect(html).toContain('Sign-in failed');
    expect(html).toMatch(/wasn't recognised|not recognised|expired/i);
  });

  test('verify endpoint flags a malformed token', async ({ request }) => {
    const r = await request.get('/api/auth/verify?token=not-a-token');
    expect(r.ok()).toBeTruthy();
    expect(await r.text()).toContain('malformed');
  });
});
