// Opt-in test: pulls a slice of corpus-builder's live R2 manifest and verifies
// the same curator pipeline works against it. Skipped unless the env var
// CURATOR_R2_LIVE=1 is set, so default `just e2e` runs stay hermetic.
//
// The R2 bucket is public-read at the URL below; it carries the manifest
// (manifest.jsonl, ~30 MB) plus content-addressed blobs at
// blobs/{sha[:2]}/{sha[2:4]}/{sha}. We fetch the first ~50 manifest lines,
// POST them to /api/curator/manifest with the R2 base, and walk the curator UI.

import { expect, test } from '@playwright/test';

const R2_BASE = process.env.CURATOR_R2_BASE ?? 'https://pub-7c5c57fd3e0842f0b147946928891d40.r2.dev';
const LIVE = process.env.CURATOR_R2_LIVE === '1';

test.describe('curator with live R2 corpus-builder data', () => {
  test.skip(!LIVE, 'set CURATOR_R2_LIVE=1 to run');

  test('fetch manifest slice and stream', async ({ request, page }) => {
    // Pull a small slice using a Range request — R2 accepts ranges and we only
    // want a few candidates.
    const manifestUrl = `${R2_BASE}/manifest.jsonl`;
    const headResp = await request.head(manifestUrl);
    expect(headResp.ok()).toBeTruthy();
    const range = await request.get(manifestUrl, {
      headers: { range: 'bytes=0-32768' },
    });
    expect([200, 206]).toContain(range.status());
    const body = (await range.text())
      .split('\n')
      .filter((l) => l.trim().length > 0 && !l.startsWith('#'))
      .slice(0, 25)
      .join('\n');
    expect(body.length).toBeGreaterThan(0);

    const load = await request.post('/api/curator/manifest', {
      data: { kind: 'jsonl', body, blob_url_base: R2_BASE },
    });
    expect(load.ok()).toBeTruthy();
    const data = await load.json();
    expect(data.inserted).toBeGreaterThan(0);

    await page.goto('/');
    await page.evaluate(() => localStorage.clear());
    await page.evaluate(() => {
      localStorage.setItem('squintly:curator_id', crypto.randomUUID());
    });
    await page.locator('.squintly-tabs button[data-tab="curator"]').click();
    // Image actually loads from R2 — check that the <img> resolves (network
    // dependent). We give it a generous timeout.
    const img = page.locator('.curator-img').first();
    await expect(img).toBeVisible({ timeout: 30_000 });
    await img.evaluate(
      (el) => new Promise((res) => {
        const i = el as HTMLImageElement;
        if (i.complete && i.naturalWidth > 0) res(null);
        else { i.addEventListener('load', () => res(null), { once: true }); i.addEventListener('error', () => res(null), { once: true }); }
      }),
    );
    const ok = await img.evaluate((el) => (el as HTMLImageElement).naturalWidth > 0);
    expect(ok).toBeTruthy();
  });
});
