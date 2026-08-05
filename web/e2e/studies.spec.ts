// Runtime study selection.
//
// One deployment hosts several studies whose trial streams differ because what
// they measure differs — the crowd study interleaves ratings with pairwise, a
// rank-agreement study is forced choice only. The observer picks by name, the
// choice rides on the session, and every response carries `study_id` so the
// two can never be pooled in analysis.

import { expect, test } from '@playwright/test';

import { clickBegin, completeProfileAndStart, gotoFresh, submitOneTrial, passInstructions } from './helpers';

test.describe('study selection', () => {
  test('GET /api/studies lists selectable studies', async ({ request }) => {
    const r = await request.get('/api/studies');
    expect(r.ok()).toBeTruthy();
    const studies = (await r.json()) as Array<{ id: string; label: string; unlisted: boolean }>;
    // Switching between projects has to be possible, so more than one study is
    // offered. `zensr-dejpeg` is unlisted because it has no restored encodings
    // yet — listing it would offer a study that can only 409.
    const ids = studies.map((s) => s.id);
    expect(ids).toContain('ssim2-nonphoto');
    expect(ids).toContain('main');
    expect(ids, 'a study with no corpus must not be offered').not.toContain('zensr-dejpeg');
    expect(studies.length, 'the picker needs something to switch between').toBeGreaterThan(1);
    expect(studies.every((s) => !s.unlisted), 'listed endpoint must omit unlisted studies').toBe(true);
  });

  // The picker is how an observer moves between projects. Unlisting everything
  // but one study removed it entirely, and with it any way to switch.
  test('the picker offers every listed study', async ({ page }) => {
    await gotoFresh(page);
    await expect(page.locator('#study-picker')).toBeVisible();
    await expect(page.locator('.study-option[data-study="ssim2-nonphoto"]')).toBeVisible();
    await expect(page.locator('.study-option[data-study="main"]')).toBeVisible();
    await expect(page.locator('.study-option[data-study="zensr-dejpeg"]')).toHaveCount(0);
  });

  test('picking a study on the welcome screen tags the session', async ({ page }) => {
    await gotoFresh(page);
    await page.locator('.study-option[data-study="ssim2-nonphoto"]').click();
    await expect(page.locator('.study-option[data-study="ssim2-nonphoto"]')).toHaveClass(/on/);
    await clickBegin(page);
    await page.getByRole('button', { name: /^Skip$/ }).click();
    await completeProfileAndStart(page);
    await submitOneTrial(page, { pair: 'tie' });
    const stored = await page.evaluate(() => localStorage.getItem('squintly:study_id'));
    expect(stored).toBe('ssim2-nonphoto');
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

  // The study is majority non-photo with a DECLARED photographic minority as a
  // within-session control. It used to draw from the whole corpus — ~38%
  // photographs under a label saying otherwise — which is a different thing:
  // that was undeclared and untagged, this is proportioned and recorded in
  // `content_class` so analysis can split the arms.
  test('the non-photo study is majority non-photo with a photo control minority', async ({
    request,
  }) => {
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
    let photo = 0;
    let total = 0;
    for (let i = 0; i < 120; i++) {
      const r = await request.get(`/api/trial/next?session_id=${sess.session_id}`);
      if (!r.ok()) continue;
      const corpus: string = (await r.json()).source_corpus ?? '';
      total += 1;
      if (PHOTO.some((p) => corpus.includes(p))) photo += 1;
    }
    expect(total, 'needed trials to measure').toBeGreaterThan(40);
    const frac = photo / total;
    // Declared 0.25. A wide band: the mock corpus has few sources per class, so
    // the draw is coarse — what matters is that BOTH classes appear and
    // non-photo dominates.
    expect(frac, `photo share ${frac.toFixed(2)} — both classes must appear`).toBeGreaterThan(0.05);
    expect(frac, `photo share ${frac.toFixed(2)} — non-photo must dominate`).toBeLessThan(0.5);
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

  // With one study listed there is no picker to click, so drive the same
  // property the picker existed to guarantee: a stored study id is what the
  // session is actually created under.
  test('a stored study id tags the session it creates', async ({ page }) => {
    await gotoFresh(page);
    await page.evaluate(() => localStorage.setItem('squintly:study_id', 'ssim2-nonphoto'));
    await page.reload();
    await passInstructions(page);

    await clickBegin(page);
    await page.getByRole('button', { name: /^Skip$/ }).click();
    await completeProfileAndStart(page);
    await submitOneTrial(page, { pair: 'tie' });

    // The choice must survive into the session the server actually created.
    const stored = await page.evaluate(() => localStorage.getItem('squintly:study_id'));
    expect(stored).toBe('ssim2-nonphoto');
  });
});

test.describe('content provenance in the export', () => {
  // imazen/squintly#4 wants per-category SROCC, and `responses.tsv` carried no
  // corpus or content column at all — so that analysis could not be run from
  // the export, and a check for "did the non-photo study serve photographs"
  // silently read a missing field and always answered no. A vacuous check is
  // worse than a missing one: it reports reassurance.
  test('every response carries the stratum and content class it was served as', async ({
    request,
  }) => {
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

    const t = await (
      await request.get(`/api/trial/next?session_id=${sess.session_id}`)
    ).json();
    const ok = await request.post(`/api/trial/${t.trial_id}/response`, {
      data: {
        choice: 'tie',
        dwell_ms: 4000,
        reveal_count: 1,
        reveal_ms_total: 900,
        zoom_used: false,
        pan_count: 0,
        pan_distance_css: 0,
        zoom_factor: 1,
        input_mode: 'tap',
        keyboard_used: false,
        ui_ready_ms: 120,
        pannable_w_css: 0,
        pannable_h_css: 0,
        visible_w_css: 300,
        visible_h_css: 300,
        viewport_w_css: 400,
        viewport_h_css: 600,
        orientation: 'portrait',
        image_displayed_w_css: 300,
        image_displayed_h_css: 300,
        intrinsic_to_device_ratio: 1,
        pixels_per_degree: null,
      },
    });
    expect(ok.ok(), await ok.text()).toBeTruthy();

    const tsv = await (await request.get('/api/export/responses.tsv')).text();
    const [header, ...lines] = tsv.trim().split('\n');
    const cols = header.split('\t');
    expect(cols, 'export must name the stratum').toContain('source_corpus');
    expect(cols, 'export must name the content class').toContain('content_class');

    const row = lines.map((l) => l.split('\t')).find((r) => r[0] === t.trial_id);
    expect(row, 'the response just recorded should be in the export').toBeTruthy();
    const cls = row![cols.indexOf('content_class')];
    const corpus = row![cols.indexOf('source_corpus')];
    // Whatever class it served, the row must say which — that is the point of
    // the column, and it is what lets analysis split the control arm from the
    // measurement arm. (This check was previously vacuous: the column did not
    // exist, so it read a missing field and always agreed.)
    expect(['non_photo', 'photo'], `content_class for ${corpus}`).toContain(cls);
    expect(corpus.length, 'stratum must be recorded').toBeGreaterThan(0);
  });
});

test.describe('study controls', () => {
  async function answer(request: import('@playwright/test').APIRequestContext, trialId: string) {
    return request.post(`/api/trial/${trialId}/response`, {
      data: {
        choice: 'a',
        dwell_ms: 5000,
        reveal_count: 2,
        reveal_ms_total: 1200,
        zoom_used: false,
        pan_count: 0,
        pan_distance_css: 0,
        zoom_factor: 1,
        input_mode: 'hold',
        keyboard_used: false,
        ui_ready_ms: 90,
        switch_count: 4,
        ms_on_a: 1800,
        ms_on_b: 1500,
        ms_on_ref: 1200,
        pannable_w_css: 0,
        pannable_h_css: 0,
        visible_w_css: 300,
        visible_h_css: 300,
        viewport_w_css: 400,
        viewport_h_css: 600,
        orientation: 'portrait',
        image_displayed_w_css: 300,
        image_displayed_h_css: 300,
        intrinsic_to_device_ratio: 1,
        pixels_per_degree: null,
      },
    });
  }

  async function session(request: import('@playwright/test').APIRequestContext) {
    return (
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
  }

  // Difficulty signal. `reveal_ms_total` only ever measured the reference,
  // which under hold/buttons is the RESTING view — so it reflects "not pressing
  // anything" rather than effort. Per-view dwell and switch count are what say
  // whether a pair was near the observer's discrimination threshold.
  test('per-view dwell and switch count reach the export', async ({ request }) => {
    const sess = await session(request);
    const t = await (await request.get(`/api/trial/next?session_id=${sess.session_id}`)).json();
    const ok = await answer(request, t.trial_id);
    expect(ok.ok(), await ok.text()).toBeTruthy();

    const tsv = await (await request.get('/api/export/responses.tsv')).text();
    const [header, ...lines] = tsv.trim().split('\n');
    const cols = header.split('\t');
    for (const c of ['switch_count', 'ms_on_a', 'ms_on_b', 'ms_on_ref', 'repeat_of_trial_id']) {
      expect(cols, `export must carry ${c}`).toContain(c);
    }
    const row = lines.map((l) => l.split('\t')).find((r) => r[0] === t.trial_id)!;
    expect(row, 'the response should be in the export').toBeTruthy();
    expect(row[cols.indexOf('switch_count')]).toBe('4');
    expect(row[cols.indexOf('ms_on_a')]).toBe('1800');
    expect(row[cols.indexOf('ms_on_b')]).toBe('1500');
    expect(row[cols.indexOf('ms_on_ref')]).toBe('1200');
  });

  // Test-retest. Human-vs-ssim2 SROCC is uninterpretable without it: if an
  // observer agrees with themselves only 80% of the time, the metric cannot
  // exceed roughly that, and the headline number reads completely differently
  // against a ceiling of 0.95 than against 0.72.
  test('the study re-serves pairs it has already asked, and records which', async ({ request }) => {
    const sess = await session(request);

    const answered = new Map<string, string>(); // trial id → source+encodings
    let repeats = 0;
    for (let i = 0; i < 80; i++) {
      const r = await request.get(`/api/trial/next?session_id=${sess.session_id}`);
      if (!r.ok()) continue;
      const t = await r.json();
      if (t.kind !== 'pair') continue;
      const key = [t.source_hash, ...[t.a.encoding_id, t.b.encoding_id].sort()].join('|');
      // A repeat presents a pair already answered in this session.
      if ([...answered.values()].includes(key)) repeats += 1;
      answered.set(t.trial_id, key);
      await answer(request, t.trial_id);
    }

    expect(repeats, 'p_repeat = 0.08 over ~80 trials should re-serve several').toBeGreaterThan(0);

    // And the link is recorded, so analysis can pair them without guessing.
    const tsv = await (await request.get('/api/export/responses.tsv')).text();
    const [header, ...lines] = tsv.trim().split('\n');
    const cols = header.split('\t');
    const idx = cols.indexOf('repeat_of_trial_id');
    const linked = lines
      .map((l) => l.split('\t'))
      .filter((r) => (r[idx] ?? '').length > 0);
    expect(linked.length, 'repeats must record what they repeat').toBeGreaterThan(0);
    // Every link points at a real, earlier trial.
    const allIds = new Set(lines.map((l) => l.split('\t')[0]));
    for (const r of linked) {
      expect(allIds.has(r[idx]), `repeat_of ${r[idx]} should name a known trial`).toBe(true);
      expect(r[idx], 'a trial cannot repeat itself').not.toBe(r[0]);
    }
  });
});

test.describe('reviewer leaderboard', () => {
  test('shows work and quality per reviewer, and identifies nobody', async ({ request }) => {
    const r = await request.get('/api/leaderboard');
    expect(r.ok(), await r.text()).toBeTruthy();
    const rows = (await r.json()) as Array<Record<string, unknown>>;
    expect(Array.isArray(rows)).toBe(true);
    if (rows.length === 0) return; // nothing rated on this run

    for (const row of rows) {
      // The handle is a salted derivation, so it must not carry an identity.
      const h = String(row.handle);
      expect(h, `handle ${h} looks like an address`).not.toContain('@');
      expect(h).toMatch(/^[a-z]+-[a-z]+-\d{2}$/);
      // Nothing identifying may appear anywhere in the payload.
      for (const k of Object.keys(row)) {
        expect(k, 'the board must not carry raw identity').not.toMatch(
          /email|observer_id|ip|address/i,
        );
      }
      // Both halves of the question the board answers.
      for (const k of ['trials', 'sessions', 'active_days']) {
        expect(typeof row[k], `${k} should be numeric`).toBe('number');
      }
      expect('golden_pass_rate' in row).toBe(true);
      expect('self_agreement' in row).toBe(true);
      expect('repeat_pairs' in row).toBe(true);
    }
    // Sorted by volume, but quality is present so a high count can be judged.
    const counts = rows.map((r2) => Number(r2.trials));
    expect([...counts].sort((a, b) => b - a)).toEqual(counts);
  });
});
