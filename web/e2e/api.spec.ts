import { expect, test } from '@playwright/test';

test.describe('HTTP API', () => {
  test('GET /api/stats returns the expected shape', async ({ request }) => {
    const r = await request.get('/api/stats');
    expect(r.ok()).toBeTruthy();
    const body = await r.json();
    for (const key of ['observers', 'sessions', 'trials', 'responses', 'manifest_sources', 'manifest_encodings']) {
      expect(body).toHaveProperty(key);
      expect(typeof body[key]).toBe('number');
    }
    expect(body.manifest_sources).toBeGreaterThan(0);
    expect(body.manifest_encodings).toBeGreaterThan(0);
  });

  test('POST /api/session round-trips streak and supported_codecs', async ({ request }) => {
    const r = await request.post('/api/session', {
      data: {
        observer_id: null,
        user_agent: 'e2e-test',
        device_pixel_ratio: 3,
        screen_width_css: 390,
        screen_height_css: 844,
        color_gamut: 'p3',
        css_px_per_mm: 4.7,
        viewing_distance_cm: 30,
        ambient_light: 'room',
        local_date: new Date().toISOString().slice(0, 10),
        supported_codecs: ['jpeg', 'webp'],
        codec_probe_cached: false,
      },
    });
    expect(r.ok()).toBeTruthy();
    const body = await r.json();
    expect(body.observer_id).toBeTruthy();
    expect(body.session_id).toBeTruthy();
    expect(body.streak_days).toBeGreaterThanOrEqual(1);
    expect(['advanced', 'frozen', 'reset', 'same_day']).toContain(body.streak_outcome);
  });

  test('GET /api/trial/next 409s when no codecs match', async ({ request }) => {
    // Create a session declaring only "png" (which the manifest never returns).
    const sess = await (
      await request.post('/api/session', {
        data: {
          observer_id: null,
          user_agent: 'e2e-test',
          device_pixel_ratio: 1,
          screen_width_css: 1000,
          screen_height_css: 1000,
          local_date: new Date().toISOString().slice(0, 10),
          supported_codecs: ['png'],
        },
      })
    ).json();
    const r = await request.get(`/api/trial/next?session_id=${sess.session_id}`);
    expect(r.status()).toBe(409);
  });

  test('export TSVs return tab-separated headers', async ({ request }) => {
    for (const path of ['/api/export/pareto.tsv', '/api/export/thresholds.tsv', '/api/export/responses.tsv']) {
      const r = await request.get(path);
      expect(r.ok()).toBeTruthy();
      const body = await r.text();
      const firstLine = body.split('\n')[0];
      expect(firstLine).toContain('\t');
    }
  });

  test('GET /api/observer/{id}/profile returns themes and badges arrays', async ({ request }) => {
    const sess = await (
      await request.post('/api/session', {
        data: {
          observer_id: null,
          user_agent: 'e2e-test',
          device_pixel_ratio: 2,
          screen_width_css: 800,
          screen_height_css: 600,
          local_date: new Date().toISOString().slice(0, 10),
          supported_codecs: ['jpeg', 'webp', 'avif'],
        },
      })
    ).json();
    const profile = await (
      await request.get(`/api/observer/${sess.observer_id}/profile`)
    ).json();
    expect(Array.isArray(profile.badges)).toBe(true);
    expect(Array.isArray(profile.themes)).toBe(true);
    expect(profile.themes.length).toBeGreaterThanOrEqual(1);
    const slugs = profile.themes.map((t: { slug: string }) => t.slug);
    expect(slugs).toContain('nature');
  });
});
