import { expect, test } from '@playwright/test';

import { gotoFresh } from './helpers';

test.describe('optional email sign-in', () => {
  test('welcome screen exposes a sign-in link', async ({ page }) => {
    await gotoFresh(page);
    await expect(page.getByText(/Already have an email-linked account/i)).toBeVisible();
  });

  test('sign-in modal opens, validates email, and reports unconfigured Postmark', async ({ page }) => {
    await gotoFresh(page);
    await page.getByText(/Already have an email-linked account/i).click();
    await expect(page.getByRole('heading', { name: /Save your progress/i })).toBeVisible();
    // Click "Send link" with no email — should warn.
    await page.getByRole('button', { name: /^Send link$/ }).click();
    await expect(page.locator('#signin-status')).toContainText(/email/i);
    // Provide an email; the test backend has no POSTMARK_SERVER_TOKEN, so the
    // start endpoint returns 503 with a clear hint and the modal surfaces it.
    await page.locator('#signin-email').fill('observer@example.com');
    await page.getByRole('button', { name: /^Send link$/ }).click();
    await expect(page.locator('#signin-status')).toContainText(/not configured|Anonymous/i);
  });

  // The test backend has no Postmark credentials, so a real request is refused
  // for being unconfigured before it ever reaches the limiter. Stub the
  // response to exercise the branch a live deployment actually takes.
  test('a rate-limited request reads as "wait", not as breakage', async ({ page }) => {
    await gotoFresh(page);
    await page.route('**/api/auth/start', (route) =>
      route.fulfill({
        status: 429,
        contentType: 'text/plain',
        headers: { 'retry-after': '47' },
        body:
          'Slow down — a sign-in link was just sent to this address; wait 47s. ' +
          'No link was sent. Anonymous use is unaffected.',
      }),
    );
    await page.getByText(/Already have an email-linked account/i).click();
    await page.locator('#signin-email').fill('observer@example.com');
    await page.getByRole('button', { name: /^Send link$/ }).click();

    const status = page.locator('#signin-status');
    await expect(status).toContainText(/slow down/i);
    await expect(status).toContainText(/47s/);
    // It must not be dressed up as a failure to send.
    await expect(status).not.toContainText(/couldn.t send/i);
  });

  // Sign-in is deliberately open: an allowlist here would lock participants out
  // of their own data on a second device.
  test('any address is accepted, not just a roster', async ({ request }) => {
    const r = await request.post('/api/auth/start', {
      data: {
        email: 'someone-random@example.com',
        observer_id: null,
        origin: 'http://127.0.0.1:18030',
      },
    });
    // 503 (no Postmark on the test backend) means it got past address checks.
    // 403 would mean an allowlist crept back in front of sign-in.
    expect(r.status(), await r.text()).toBe(503);
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
