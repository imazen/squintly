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

    // Trial type is sampler-chosen, so the first one may be a pair. Advance
    // until a single-stimulus trial appears rather than skipping the test:
    // a runtime self-skip is a silent pass that asserts nothing (global
    // CLAUDE.md, "NO GRACEFUL SKIPS"). If none shows up in 15 trials the
    // sampler is broken and this must fail loudly.
    let found = false;
    for (let i = 0; i < 15; i++) {
      if (await page.locator('.rating-panel').isVisible()) {
        found = true;
        break;
      }
      await submitOneTrial(page, { pair: 'tie' });
      await awaitAnyTrialPanel(page);
    }
    expect(found, 'no single-stimulus trial served in 15 trials').toBe(true);

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
