// Layout regression tests on Galaxy Z Fold 7 viewports. Skips itself unless
// the project name is one of the zfold7-* projects defined in
// playwright.config.ts.

import { expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

import { COEFFICIENT_PORT } from '../playwright.config';

const HERE = dirname(fileURLToPath(import.meta.url));
const FIXTURE_BODY = readFileSync(resolve(HERE, 'curator-fixture.jsonl'), 'utf-8');
const BLOB_BASE = `http://127.0.0.1:${COEFFICIENT_PORT}`;

test.describe('curator on Z Fold 7', () => {
  test.beforeEach(async ({ request, page }, testInfo) => {
    test.skip(!testInfo.project.name.startsWith('zfold7-'), 'Z Fold project only');
    const r = await request.post('/api/curator/manifest', {
      data: { kind: 'jsonl', body: FIXTURE_BODY, blob_url_base: BLOB_BASE },
    });
    expect(r.ok()).toBeTruthy();
    await page.goto('/');
    await page.evaluate(() => localStorage.clear());
    await page.evaluate(() => {
      localStorage.setItem('squintly:curator_id', crypto.randomUUID());
    });
  });

  test('cover-display layout: stream and curate stack vertically', async ({ page }, testInfo) => {
    test.skip(testInfo.project.name !== 'zfold7-cover', 'cover only');
    await page.goto('/');
    await page.locator('.squintly-tabs button[data-tab="curator"]').click();
    // Single-column layout — confirm the curator-meta sits below (not beside) the viewport.
    const viewportBox = await page.locator('.curator-viewport').boundingBox();
    const metaBox = await page.locator('.curator-meta').boundingBox();
    expect(viewportBox && metaBox).toBeTruthy();
    expect(metaBox!.y).toBeGreaterThan(viewportBox!.y + viewportBox!.height - 4);
    // Take/skip buttons stay reachable in the bottom third of the viewport.
    const takeBtn = page.locator('#take');
    await expect(takeBtn).toBeVisible();
    const tb = await takeBtn.boundingBox();
    const screenH = page.viewportSize()!.height;
    expect(tb!.y).toBeGreaterThan(screenH / 2);
  });

  test('inner-display layout: curate uses side-by-side breakpoint', async ({ page }, testInfo) => {
    test.skip(testInfo.project.name !== 'zfold7-inner', 'inner only');
    await page.goto('/');
    await page.locator('.squintly-tabs button[data-tab="curator"]').click();
    await page.locator('#take').click();
    await expect(page.locator('[data-screen="curate"]')).toBeVisible();
    // The CSS @media (min-width: 720px) and (orientation: portrait) puts the
    // preview to the left of the controls. Verify the preview sits to the left
    // of the groups grid.
    const preview = await page.locator('.curator-preview').boundingBox();
    const groups = await page.locator('.curator-groups').boundingBox();
    expect(preview && groups).toBeTruthy();
    expect(preview!.x + preview!.width / 2).toBeLessThan(groups!.x + groups!.width / 2);
  });
});
