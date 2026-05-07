// Curator-mode end-to-end test.
//
// Loads the corpus-builder-shaped JSONL fixture into Squintly via
// /api/curator/manifest, then drives the UI through Stream → Curate →
// Threshold and verifies the saved threshold round-trips through the export
// endpoint.

import { expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

import { COEFFICIENT_PORT } from '../playwright.config';

const HERE = dirname(fileURLToPath(import.meta.url));
const FIXTURE_PATH = resolve(HERE, 'curator-fixture.jsonl');
const FIXTURE_BODY = readFileSync(FIXTURE_PATH, 'utf-8');
const BLOB_BASE = `http://127.0.0.1:${COEFFICIENT_PORT}`;

test.describe('curator mode', () => {
  test.beforeEach(async ({ request, page }) => {
    // Load the candidate manifest into the DB before each test. Idempotent
    // — the upsert keeps the same set on repeat calls.
    const r = await request.post('/api/curator/manifest', {
      data: { kind: 'jsonl', body: FIXTURE_BODY, blob_url_base: BLOB_BASE },
    });
    expect(r.ok()).toBeTruthy();
    // Use a fresh curator UUID per test so streams don't leak.
    await page.goto('/');
    await page.evaluate(() => localStorage.clear());
    await page.evaluate(() => {
      localStorage.setItem('squintly:curator_id', crypto.randomUUID());
    });
  });

  test('welcome screen surfaces license credits and tab bar', async ({ page }) => {
    await page.goto('/');
    await expect(page.getByRole('heading', { name: /Image Discrimination Study/i })).toBeVisible();
    await expect(page.locator('.squintly-tabs button[data-tab="curator"]')).toBeVisible();
    await expect(page.locator('.squintly-tabs button[data-tab="rate"]')).toBeVisible();
    await expect(page.locator('.squintly-tabs button[data-tab="calibrate"]')).toBeVisible();
    // License credits panel is collapsed but present.
    await expect(page.locator('.credits summary')).toContainText(/Image sources/i);
    await page.locator('.credits summary').click();
    await expect(page.locator('.credits-table')).toBeVisible();
    // Specifically Unsplash, Wikimedia, and the mixed-research fallback are listed.
    await expect(page.locator('a[data-license-id="unsplash"]')).toBeVisible();
    await expect(page.locator('a[data-license-id="wikimedia-mixed"]')).toBeVisible();
    await expect(page.locator('a[data-license-id="mixed-research"]')).toBeVisible();
  });

  test('stream renders first candidate with license badge and corpus label', async ({ page }) => {
    await page.goto('/');
    await page.locator('.squintly-tabs button[data-tab="curator"]').click();
    await expect(page.locator('[data-screen="stream"]')).toBeVisible();
    await expect(page.locator('.curator-corpus')).toContainText(/unsplash-webp|source_jpegs|wikimedia-webshapes/);
    // License badge visible and clickable; data-license-id matches one of the
    // three corpora's policies.
    const badge = page.locator('.curator-license-badge').first();
    await expect(badge).toBeVisible();
    const licId = await badge.getAttribute('data-license-id');
    expect(['unsplash', 'wikimedia-mixed', 'mixed-research']).toContain(licId);
  });

  test('reject advances to next candidate without saving threshold', async ({ page, request }) => {
    const curatorId = await page.evaluate(() => localStorage.getItem('squintly:curator_id'));
    expect(curatorId).toBeTruthy();
    await page.goto('/');
    await page.locator('.squintly-tabs button[data-tab="curator"]').click();
    await expect(page.locator('.curator-corpus')).toBeVisible();
    const firstCorpus = await page.locator('.curator-corpus').textContent();
    await page.locator('#reject').click();
    await expect(page.locator('.curator-corpus')).not.toHaveText(firstCorpus ?? '');
    // Progress reflects the rejection.
    const prog = await request.get(`/api/curator/progress?curator_id=${curatorId}`);
    const data = await prog.json();
    expect(data.rejects).toBeGreaterThanOrEqual(1);
  });

  test('take → curate → threshold round-trip persists to export.tsv', async ({ page, request }) => {
    const curatorId = await page.evaluate(() => localStorage.getItem('squintly:curator_id'));
    await page.goto('/');
    await page.locator('.squintly-tabs button[data-tab="curator"]').click();
    await expect(page.locator('.curator-corpus')).toBeVisible();
    await page.locator('#take').click();
    await expect(page.locator('[data-screen="curate"]')).toBeVisible();
    // Toggle the core_zensim group on (default may already be). Pick at least
    // one size chip.
    await page.locator('.curator-group-btn[data-group="core_zensim"]').click();
    // If the suggestion already had it, this toggles off; click again to turn on.
    const isOn = await page
      .locator('.curator-group-btn[data-group="core_zensim"].on')
      .count();
    if (!isOn) {
      await page.locator('.curator-group-btn[data-group="core_zensim"]').click();
    }
    // Pick the smallest enabled chip so the test is fast.
    const firstEnabled = page.locator('.curator-chip:not([disabled])').first();
    await firstEnabled.click();
    // Find threshold.
    await page.locator('#find-thr').click();
    await expect(page.locator('[data-screen="threshold"]')).toBeVisible();
    // Slider — drag to a known position, save.
    const slider = page.locator('#qslider');
    await slider.evaluate((el) => {
      const inp = el as HTMLInputElement;
      inp.value = '72';
      inp.dispatchEvent(new Event('input', { bubbles: true }));
      inp.dispatchEvent(new Event('change', { bubbles: true }));
    });
    await expect(page.locator('#qval')).toHaveText('q = 72');
    await page.locator('#save-thr').click();
    await expect(page.locator('[data-screen="stream"]')).toBeVisible();

    // Validate via export endpoint. Header + at least one row with q=72 and a
    // license id we know.
    const tsvResp = await request.get(`/api/curator/export.tsv?curator_id=${curatorId}`);
    expect(tsvResp.ok()).toBeTruthy();
    const tsv = await tsvResp.text();
    expect(tsv.split('\n')[0]).toContain('license_id');
    expect(tsv).toMatch(/72\.00/);
    expect(tsv).toMatch(/(unsplash|wikimedia-mixed|mixed-research)/);
    // Encoder identity is recorded so threshold rows are attributable.
    expect(tsv).toContain('browser-canvas-jpeg');
  });

  test('exit button returns to welcome with curator progress summary', async ({ page }) => {
    await page.goto('/');
    await page.locator('.squintly-tabs button[data-tab="curator"]').click();
    await page.locator('#reject').click();
    await page.locator('.curator-exit').click();
    await expect(page.getByRole('heading', { name: /Image Discrimination Study/i })).toBeVisible();
    await expect(page.locator('.curator-progress-summary')).toBeVisible();
  });
});
