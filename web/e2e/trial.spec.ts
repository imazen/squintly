import { expect, test } from '@playwright/test';

import {
  awaitAnyTrialPanel,
  clickBegin,
  completeProfileAndStart,
  gotoFresh,
  ratePair,
  rateSingle,
  submitOneTrial,
} from './helpers';

test.describe('trial loop', () => {
  test('records a rating and advances to the next trial', async ({ page, request }) => {
    const before = await (await request.get('/api/stats')).json();
    await gotoFresh(page);
    await clickBegin(page);
    await page.getByRole('button', { name: /^Skip$/ }).click();
    await completeProfileAndStart(page);

    await submitOneTrial(page);
    // The next trial should mount within a couple of seconds.
    await awaitAnyTrialPanel(page);

    const after = await (await request.get('/api/stats')).json();
    expect(after.responses).toBeGreaterThan(before.responses);
    expect(after.sessions).toBeGreaterThan(before.sessions);
  });

  test('hold-to-reveal swaps to reference image on single-stimulus trials', async ({ page }) => {
    await gotoFresh(page);
    await clickBegin(page);
    await page.getByRole('button', { name: /^Skip$/ }).click();
    await completeProfileAndStart(page);
    await awaitAnyTrialPanel(page);

    if (!(await page.locator('.rating-panel').isVisible())) {
      // pair trial — skip; we'll get a single eventually but this test is single-only.
      test.skip(true, 'first trial happened to be a pair');
    }

    const img = page.locator('#stimulus');
    const initialSrc = await img.getAttribute('src');
    const viewport = page.locator('#viewport');
    await viewport.dispatchEvent('pointerdown');
    // After pointerdown the src should switch to the source URL.
    await expect.poll(async () => img.getAttribute('src')).not.toBe(initialSrc);
    await viewport.dispatchEvent('pointerup');
    await expect.poll(async () => img.getAttribute('src')).toBe(initialSrc);
  });

  test('rating ten trials awards the first_10 milestone badge', async ({ page, request }) => {
    await gotoFresh(page);
    await clickBegin(page);
    await page.getByRole('button', { name: /^Skip$/ }).click();
    await completeProfileAndStart(page);

    for (let i = 0; i < 10; i++) {
      await submitOneTrial(page);
    }
    // Pull the observer id straight from localStorage, then GET the profile.
    const observerId = await page.evaluate(() => localStorage.getItem('squintly:observer_id'));
    expect(observerId).not.toBeNull();
    const profile = await (await request.get(`/api/observer/${observerId}/profile`)).json();
    expect(profile.total_trials).toBeGreaterThanOrEqual(10);
    const slugs = (profile.badges as Array<{ slug: string }>).map((b) => b.slug);
    expect(slugs).toContain('first_10');
  });
});
