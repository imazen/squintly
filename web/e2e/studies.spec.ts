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

  // The study's name is a claim about its data. It constrained only the trial
  // mix for a while, so it served forced-choice trials over the whole corpus —
  // photographic strata included — which is valid data filed under the wrong
  // question. The mock carries real stratum names so this is checkable.
  test('the non-photo study never serves a photographic source', async ({ request }) => {
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

    const PHOTO = ['1400-lilith-nature', '2000-unsplash-people'];
    const seen = new Set<string>();
    for (let i = 0; i < 30; i++) {
      const r = await request.get(`/api/trial/next?session_id=${sess.session_id}`);
      if (!r.ok()) continue;
      const t = await r.json();
      const corpus: string = t.source_corpus ?? '';
      for (const p of PHOTO) {
        expect(corpus.includes(p), `non-photo study served ${corpus}`).toBe(false);
      }
      seen.add(corpus);
    }
    expect(seen.size, 'expected some trials to compare against').toBeGreaterThan(0);
    // ...and it must still reach more than one non-photo stratum.
    expect(seen.size).toBeGreaterThan(1);
  });

  // The default study is unrestricted, so it must still reach the photographic
  // strata — a content filter leaking onto every study would silently narrow
  // the main crowd study.
  test('the main study still draws from photographic strata', async ({ request }) => {
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
          study_id: 'main',
        },
      })
    ).json();

    let sawPhoto = false;
    for (let i = 0; i < 40 && !sawPhoto; i++) {
      const r = await request.get(`/api/trial/next?session_id=${sess.session_id}`);
      if (!r.ok()) continue;
      const corpus: string = (await r.json()).source_corpus ?? '';
      if (corpus.includes('1400-lilith-nature') || corpus.includes('2000-unsplash-people')) {
        sawPhoto = true;
      }
    }
    expect(sawPhoto, 'the unrestricted study must still see photographs').toBe(true);
  });

  // Position counterbalancing, at the endpoint the observer actually hits.
  //
  // `try_pair` builds `(sorted[i], sorted[i+1])` from a quality-ascending list,
  // so slot B held the better image on every trial — 60/60 measured against the
  // live deployment. That makes "which is closer to the original" have a
  // constant answer, and no downstream fit can tell a quality judgement from a
  // side preference afterwards. The unit tests cover the swap helper; this
  // covers the wiring, which is what was actually missing.
  test('the better image is not always in the same slot', async ({ request }) => {
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

    let bBetter = 0;
    let total = 0;
    for (let i = 0; i < 80; i++) {
      const r = await request.get(`/api/trial/next?session_id=${sess.session_id}`);
      if (!r.ok()) continue;
      const t = await r.json();
      if (t.kind !== 'pair' || t.a?.quality == null || t.b?.quality == null) continue;
      total += 1;
      if (t.b.quality > t.a.quality) bBetter += 1;
    }

    expect(total, 'needed pair trials to measure').toBeGreaterThan(20);
    const frac = bBetter / total;
    // Binomial(n>=20, 0.5). A generous band keeps this from flaking while still
    // failing hard on the actual bug, which sat at 1.00.
    expect(
      frac,
      `the better image was in slot B ${(frac * 100).toFixed(0)}% of ${total} trials`,
    ).toBeGreaterThan(0.2);
    expect(frac).toBeLessThan(0.8);
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
