// Runtime study selection.
//
// One deployment hosts several studies whose trial streams differ because what
// they measure differs — the crowd study interleaves ratings with pairwise, a
// rank-agreement study is forced choice only. The observer picks by name, the
// choice rides on the session, and every response carries `study_id` so the
// two can never be pooled in analysis.

import { expect, test } from '@playwright/test';

import { clickBegin, completeProfileAndStart, gotoFresh, submitOneTrial } from './helpers';

test.describe('study selection', () => {
  test('GET /api/studies lists selectable studies', async ({ request }) => {
    const r = await request.get('/api/studies');
    expect(r.ok()).toBeTruthy();
    const studies = (await r.json()) as Array<{ id: string; label: string; unlisted: boolean }>;
    expect(studies.length).toBeGreaterThan(1);
    expect(studies.map((s) => s.id)).toContain('main');
    expect(studies.every((s) => !s.unlisted), 'listed endpoint must omit unlisted studies').toBe(true);
  });

  test('an unknown study_id is rejected, not silently substituted', async ({ request }) => {
    const r = await request.post('/api/session', {
      data: {
        observer_id: null,
        user_agent: 'e2e',
        device_pixel_ratio: 2,
        screen_width_css: 400,
        screen_height_css: 800,
        local_date: new Date().toISOString().slice(0, 10),
        supported_codecs: ['jpeg'],
        study_id: 'no-such-study',
      },
    });
    // Running a different protocol than the caller asked for would put
    // incompatible data in one table; a 400 is the correct outcome.
    expect(r.status()).toBe(400);
  });

  test('the forced-choice study serves only pairwise trials', async ({ request }) => {
    const sess = await (
      await request.post('/api/session', {
        data: {
          observer_id: null,
          user_agent: 'e2e',
          device_pixel_ratio: 2,
          screen_width_css: 400,
          screen_height_css: 800,
          local_date: new Date().toISOString().slice(0, 10),
          supported_codecs: ['jpeg', 'webp'],
          study_id: 'ssim2-nonphoto',
        },
      })
    ).json();
    expect(sess.study_id).toBe('ssim2-nonphoto');

    const kinds = new Set<string>();
    for (let i = 0; i < 25; i++) {
      const r = await request.get(`/api/trial/next?session_id=${sess.session_id}`);
      if (!r.ok()) continue; // a 409 is legitimate: no non-trivial pair available
      kinds.add((await r.json()).kind);
    }
    expect(kinds.has('single'), 'a forced-choice study must never serve a rating').toBe(false);
    expect(kinds.has('pair')).toBe(true);
  });

  test('picking a study on the welcome screen tags the session', async ({ page }) => {
    await gotoFresh(page);
    const picker = page.locator('#study-picker');
    await expect(picker).toBeVisible();

    await page.locator('.study-option[data-study="ssim2-nonphoto"]').click();
    await expect(page.locator('.study-option[data-study="ssim2-nonphoto"]')).toHaveClass(/on/);

    await clickBegin(page);
    await page.getByRole('button', { name: /^Skip$/ }).click();
    await completeProfileAndStart(page);
    await submitOneTrial(page, { pair: 'tie' });

    // The choice must survive into the session the server actually created.
    const stored = await page.evaluate(() => localStorage.getItem('squintly:study_id'));
    expect(stored).toBe('ssim2-nonphoto');
  });
});
