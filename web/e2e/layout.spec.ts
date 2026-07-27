// Layout-viewport regression guard, all projects.
//
// The class of bug this pins down: an element whose min-content width exceeds
// the device width (nowrap flex row, bare-1fr grid, unbreakable token, wide
// table) makes mobile Chrome widen the layout viewport to fit ("shrink-to-fit
// ICB"). The page then pans sideways and tap coordinates land on the wrong
// controls — on the Z Fold 7 cover display this made the curator exit button
// untappable. Assert on every major screen: layout width == device width and
// no horizontal scroll.

import { expect, test, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

import { COEFFICIENT_PORT } from '../playwright.config';
import { clickBegin, completeProfileAndStart, gotoFresh } from './helpers';

const HERE = dirname(fileURLToPath(import.meta.url));
const FIXTURE_BODY = readFileSync(resolve(HERE, 'curator-fixture.jsonl'), 'utf-8');
const BLOB_BASE = `http://127.0.0.1:${COEFFICIENT_PORT}`;

async function expectNoViewportExpansion(page: Page, screen: string) {
  const m = await page.evaluate(() => {
    const vw = window.innerWidth;
    // Elements painted past the right edge would be unreachable (the root
    // clips horizontal overflow), so catch them directly — this stays
    // meaningful even though overflow-x: clip pins scrollWidth to the
    // viewport.
    const jutting: string[] = [];
    const insideHScroller = (el: HTMLElement): boolean => {
      for (let p = el.parentElement; p; p = p.parentElement) {
        const o = getComputedStyle(p).overflowX;
        if (o === 'auto' || o === 'scroll') return true;
      }
      return false;
    };
    document.querySelectorAll<HTMLElement>('body *').forEach((el) => {
      const r = el.getBoundingClientRect();
      const cs = getComputedStyle(el);
      if (cs.display === 'none' || cs.visibility === 'hidden') return;
      if (el.closest('details:not([open])')) return;
      // Content inside a deliberate horizontal scroll container (thumbnail
      // strips) is reachable by scrolling that container — not clipped.
      if (insideHScroller(el)) return;
      if (r.width > 0 && r.right > vw + 1) {
        const id = el.id ? `#${el.id}` : '';
        jutting.push(`${el.tagName.toLowerCase()}${id} right=${Math.round(r.right)}`);
      }
    });
    return {
      innerWidth: vw,
      scrollWidth: (document.scrollingElement ?? document.documentElement).scrollWidth,
      jutting: jutting.slice(0, 10),
    };
  });
  const device = page.viewportSize()!.width;
  expect(m.innerWidth, `${screen}: layout viewport expanded (shrink-to-fit)`).toBe(device);
  expect(m.scrollWidth, `${screen}: page scrolls horizontally`).toBeLessThanOrEqual(device + 1);
  expect(m.jutting, `${screen}: elements clipped past the right edge`).toEqual([]);
}

test.describe('no horizontal overflow on any screen', () => {
  test('welcome, credits, profile, trial', async ({ page }) => {
    await gotoFresh(page);
    await expectNoViewportExpansion(page, 'welcome');

    await page.locator('.credits summary').click();
    await expect(page.locator('.credits-table')).toBeVisible();
    await expectNoViewportExpansion(page, 'welcome-credits-open');

    await clickBegin(page);
    await expectNoViewportExpansion(page, 'calibration');
    await page.getByRole('button', { name: /^Skip$/ }).click();
    await expectNoViewportExpansion(page, 'profile');
    await completeProfileAndStart(page);

    await page.waitForSelector('.rating-panel, .pair-panel', { timeout: 10_000 });
    await expectNoViewportExpansion(page, 'trial');
  });

  test('suggest form', async ({ page }) => {
    await page.goto('/');
    await page.locator('.squintly-tabs button[data-tab="suggest"]').click();
    await expect(page.locator('[data-screen="suggest"]')).toBeVisible();
    await expectNoViewportExpansion(page, 'suggest');
  });

  test('curator stream, curate, threshold', async ({ page, request }) => {
    const r = await request.post('/api/curator/manifest', {
      data: { kind: 'jsonl', body: FIXTURE_BODY, blob_url_base: BLOB_BASE },
    });
    expect(r.ok()).toBeTruthy();
    await page.goto('/');
    await page.evaluate(() => {
      localStorage.clear();
      localStorage.setItem('squintly:curator_id', crypto.randomUUID());
    });
    await page.locator('.squintly-tabs button[data-tab="curator"]').click();
    await expect(page.locator('[data-screen="stream"]')).toBeVisible();
    await expect(page.locator('.curator-meta-row')).toBeVisible();
    await expectNoViewportExpansion(page, 'curator-stream');

    await page.locator('#take').click();
    await expect(page.locator('[data-screen="curate"]')).toBeVisible();
    await expectNoViewportExpansion(page, 'curator-curate');

    // Open the thumbnail preview strip — its status line once embedded an
    // unbreakable blob URL that widened the viewport on phones.
    await page.locator('.curator-chip:not([disabled])').first().click();
    await page.locator('#preview-wrap summary').click();
    await page.waitForTimeout(300);
    await expectNoViewportExpansion(page, 'curator-curate-preview-strip');

    await page.locator('#find-thr').click();
    await expect(page.locator('[data-screen="threshold"]')).toBeVisible();
    await expectNoViewportExpansion(page, 'curator-threshold');
  });
});
