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
  // for being unconfigured before it ever reaches the allowlist. Stub the
  // response to exercise the branch a live deployment actually takes.
  test('an address off the allowlist is refused as policy, not as breakage', async ({ page }) => {
    await gotoFresh(page);
    await page.route('**/api/auth/start', (route) =>
      route.fulfill({
        status: 403,
        contentType: 'text/plain',
        body:
          "stranger@example.com is not on this deployment's sign-in allowlist, so no link " +
          'was sent. Operators: set SQUINTLY_LOGIN_ALLOWLIST.',
      }),
    );
    await page.getByText(/Already have an email-linked account/i).click();
    await page.locator('#signin-email').fill('stranger@example.com');
    await page.getByRole('button', { name: /^Send link$/ }).click();

    const status = page.locator('#signin-status');
    await expect(status).toContainText(/can.t sign in|anonymous use is unaffected/i);
    // The operator-facing env-var hint must not leak into the visitor's view.
    await expect(status).not.toContainText(/SQUINTLY_LOGIN_ALLOWLIST/);
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
