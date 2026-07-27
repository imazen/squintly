// Interactive UX audit / demo-user driver.
//
// Boots nothing itself — point it at a running squintly (see justfile
// `audit-serve`). Drives a scripted demo user through every screen at a set
// of device viewports (Z Fold 7 cover + inner, Pixel 7, desktop), captures a
// screenshot per screen, and runs three geometry diagnostics on each:
//
//   1. horizontal-overflow: document scrollWidth vs viewport width (a page
//      that scrolls sideways on a phone is always a layout bug here);
//   2. tap-interception: for every visible interactive element, scroll it
//      into view and check `elementFromPoint(center)` actually hits it (the
//      generalized form of the zfold7-cover failures where the tab bar ate
//      the exit button's taps);
//   3. tiny-targets: interactive elements smaller than 40×40 CSS px are
//      reported (report-only; some compact controls are deliberate).
//
// Output: <out>/<viewport>/<NN-screen>.png + audit-report.json + REPORT.md.
//
//   npx tsx scripts/ux-audit.ts                       # all viewports
//   AUDIT_VIEWPORTS=zfold7-cover npx tsx scripts/ux-audit.ts
//   AUDIT_BASE_URL=http://127.0.0.1:18130 AUDIT_OUT=~/tmp/audit npx tsx ...

import { chromium, type Browser, type Page } from '@playwright/test';
import { mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const BASE_URL = process.env.AUDIT_BASE_URL ?? 'http://127.0.0.1:18130';
const OUT_ROOT = expandHome(
  process.env.AUDIT_OUT ?? `/mnt/v/output/squintly/ux-audit-${new Date().toISOString().slice(0, 10)}`,
);

interface ViewportDef {
  name: string;
  width: number;
  height: number;
  deviceScaleFactor: number;
  isMobile: boolean;
}

// Keep in sync with playwright.config.ts device projects.
const ALL_VIEWPORTS: ViewportDef[] = [
  { name: 'zfold7-cover', width: 304, height: 772, deviceScaleFactor: 3, isMobile: true },
  { name: 'zfold7-inner', width: 749, height: 832, deviceScaleFactor: 2.625, isMobile: true },
  { name: 'pixel7', width: 412, height: 915, deviceScaleFactor: 2.625, isMobile: true },
  { name: 'desktop', width: 1280, height: 800, deviceScaleFactor: 1, isMobile: false },
];

const pick = (process.env.AUDIT_VIEWPORTS ?? '').split(',').filter(Boolean);
const VIEWPORTS = pick.length ? ALL_VIEWPORTS.filter((v) => pick.includes(v.name)) : ALL_VIEWPORTS;

interface Interception {
  selector: string;
  text: string;
  hitDescription: string;
}

interface ScreenReport {
  screen: string;
  file: string;
  scrollWidth: number;
  viewportWidth: number;
  horizontalOverflow: boolean;
  interceptions: Interception[];
  tinyTargets: string[];
}

interface ViewportReport {
  viewport: ViewportDef;
  screens: ScreenReport[];
}

function expandHome(p: string): string {
  return p.startsWith('~/') ? join(homedir(), p.slice(2)) : p;
}

async function diagnose(page: Page): Promise<Omit<ScreenReport, 'screen' | 'file'>> {
  return page.evaluate(() => {
    const doc = document.scrollingElement ?? document.documentElement;
    const vw = window.innerWidth;
    const scrollWidth = doc.scrollWidth;

    const describe = (el: Element): string => {
      const id = el.id ? `#${el.id}` : '';
      const cls = el.classList.length ? `.${[...el.classList].slice(0, 2).join('.')}` : '';
      return `${el.tagName.toLowerCase()}${id}${cls}`;
    };

    // When a modal scrim (or the curator peek overlay) is up, background
    // elements are legitimately covered — scope the check to the overlay.
    const overlay = document.querySelector<HTMLElement>('.scrim, .curator-peek-overlay');
    const scope: ParentNode = overlay ?? document;
    const interactive = [
      ...scope.querySelectorAll<HTMLElement>(
        'button, a[href], summary, input, select, textarea, [role="button"]',
      ),
    ].filter((el) => {
      const r = el.getBoundingClientRect();
      if (r.width === 0 || r.height === 0) return false;
      const style = getComputedStyle(el);
      if (style.visibility === 'hidden' || style.display === 'none' || el.closest('[hidden]')) {
        return false;
      }
      // Content inside a closed <details> isn't interactable, but Chromium
      // still reports speculative layout boxes for it (content-visibility) —
      // skip everything but the <summary> itself.
      const details = el.closest('details:not([open])');
      if (details && el.tagName !== 'SUMMARY') return false;
      return true;
    });

    const interceptions: Interception[] = [];
    const tinyTargets: string[] = [];

    for (const el of interactive) {
      el.scrollIntoView({ block: 'center', inline: 'nearest' });
      // Inline elements wrapped onto multiple lines have a bounding box whose
      // center can fall in the gap between line boxes — measure the first
      // line box instead, like a real tap on the link text.
      const rects = el.getClientRects();
      const r = rects.length > 1 ? rects[0] : el.getBoundingClientRect();
      const cx = r.x + r.width / 2;
      const cy = r.y + r.height / 2;
      if (r.width < 40 || r.height < 40) {
        tinyTargets.push(`${describe(el)} ${Math.round(r.width)}×${Math.round(r.height)}`);
      }
      if (cx < 0 || cy < 0 || cx > vw || cy > window.innerHeight) {
        interceptions.push({
          selector: describe(el),
          text: (el.textContent ?? '').trim().slice(0, 40),
          hitDescription: `center off-viewport at (${Math.round(cx)}, ${Math.round(cy)})`,
        });
        continue;
      }
      const hit = document.elementFromPoint(cx, cy);
      if (!hit) continue;
      const ok =
        hit === el ||
        el.contains(hit) ||
        (hit instanceof HTMLLabelElement && hit.control === el) ||
        (el instanceof HTMLInputElement && hit.contains(el));
      if (!ok) {
        interceptions.push({
          selector: describe(el),
          text: (el.textContent ?? '').trim().slice(0, 40),
          hitDescription: describe(hit),
        });
      }
    }
    window.scrollTo(0, 0);
    return {
      scrollWidth,
      viewportWidth: vw,
      horizontalOverflow: scrollWidth > vw + 1,
      interceptions,
      tinyTargets,
    };
  });
}

/** Wait until the trial stimulus image finished decoding, so screenshots
 * show pixels rather than a mid-load black viewport. */
async function waitForStimulus(page: Page): Promise<void> {
  await page
    .waitForFunction(
      () => {
        const img = document.querySelector<HTMLImageElement>('#stimulus');
        return !!img && img.complete && img.naturalWidth > 0;
      },
      undefined,
      { timeout: 8_000 },
    )
    .catch(() => {});
  await page.waitForTimeout(150); // one frame for the onload sizing to apply
}

/** Wait until the threshold split's left canvas holds non-black pixels —
 * the browser-canvas anchor pre-encode takes a few seconds on big sources. */
async function waitForThresholdPaint(page: Page): Promise<void> {
  await page
    .waitForFunction(
      () => {
        const c = document.querySelector<HTMLCanvasElement>('#left');
        if (!c || c.width < 2) return false;
        try {
          const d = c.getContext('2d')!.getImageData(Math.floor(c.width / 2), Math.floor(c.height / 2), 1, 1).data;
          return d[3] > 0 && d[0] + d[1] + d[2] > 10;
        } catch {
          return false;
        }
      },
      undefined,
      { timeout: 10_000 },
    )
    .catch(() => {});
}

async function snap(
  page: Page,
  dir: string,
  report: ViewportReport,
  index: number,
  screen: string,
  opts: { fullPage?: boolean } = {},
): Promise<void> {
  const file = `${String(index).padStart(2, '0')}-${screen}.png`;
  // fullPage stitching renders canvases black in Chromium — threshold shots
  // use plain viewport capture.
  await page.screenshot({ path: join(dir, file), fullPage: opts.fullPage ?? true });
  const diag = await diagnose(page);
  report.screens.push({ screen, file, ...diag });
  const overflowNote = diag.horizontalOverflow
    ? ` OVERFLOW ${diag.scrollWidth}>${diag.viewportWidth}`
    : '';
  const interceptNote = diag.interceptions.length ? ` INTERCEPTS ${diag.interceptions.length}` : '';
  console.log(`  [${report.viewport.name}] ${file}${overflowNote}${interceptNote}`);
}

async function auditViewport(browser: Browser, vp: ViewportDef): Promise<ViewportReport> {
  const dir = join(OUT_ROOT, vp.name);
  // Clear stale shots: screens are captured conditionally (pair trials are
  // stochastic), so leftovers from a previous run would masquerade as
  // current state.
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
  const context = await browser.newContext({
    baseURL: BASE_URL,
    viewport: { width: vp.width, height: vp.height },
    deviceScaleFactor: vp.deviceScaleFactor,
    isMobile: vp.isMobile,
    hasTouch: vp.isMobile,
  });
  const page = await context.newPage();
  // tsx/esbuild injects `__name(...)` helpers into functions it transpiles;
  // page.evaluate serializes them into the browser where the helper doesn't
  // exist. Shim it to identity so evaluate callbacks run untouched.
  await page.addInitScript(() => {
    (window as unknown as { __name: (f: unknown, n?: string) => unknown }).__name = (f) => f;
  });
  const report: ViewportReport = { viewport: vp, screens: [] };
  let i = 0;

  // ---- Welcome + modals ----
  await page.goto('/');
  await page.evaluate(() => localStorage.clear());
  await page.reload();
  await page.waitForSelector('[data-screen="welcome"]');
  await snap(page, dir, report, ++i, 'welcome');

  await page.locator('.credits summary').click({ timeout: 5000 }).catch(() => {});
  await page.waitForTimeout(300);
  await snap(page, dir, report, ++i, 'welcome-credits-open');

  await page.locator('#signin-link').click({ timeout: 5000 }).catch(() => {});
  await page.waitForTimeout(300);
  await snap(page, dir, report, ++i, 'signin-modal');
  await page.keyboard.press('Escape').catch(() => {});
  await page.locator('.scrim #signin-cancel, .scrim button:has-text("Cancel")').first().click({ timeout: 2000 }).catch(() => {});
  await page.reload();

  // ---- Calibration (card slider → blind spot) ----
  await page.getByRole('button', { name: /^Begin$/ }).click();
  await page.waitForSelector('#slider', { timeout: 5000 }).catch(() => {});
  await snap(page, dir, report, ++i, 'calibration-card');
  const slider = page.locator('#slider');
  if (await slider.count()) {
    await slider.evaluate((el) => {
      (el as HTMLInputElement).value = '300';
      el.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await page.getByRole('button', { name: /Looks right/i }).click();
    await page.waitForTimeout(300);
    await snap(page, dir, report, ++i, 'calibration-blindspot');
    await page.locator('#skip2').click({ timeout: 5000 }).catch(() => {});
  }

  // ---- Profile form ----
  await page.waitForSelector('[data-group="ambient_light"]', { timeout: 5000 }).catch(() => {});
  await snap(page, dir, report, ++i, 'profile');
  await page.getByRole('button', { name: /^room$/ }).click({ timeout: 5000 }).catch(() => {});
  await page.getByRole('button', { name: /^no$/ }).click({ timeout: 5000 }).catch(() => {});
  await page.getByRole('button', { name: /^25-35$/ }).click({ timeout: 5000 }).catch(() => {});
  await page.getByRole('button', { name: /Start rating/i }).click({ timeout: 5000 }).catch(() => {});

  // ---- Trials: single (reveal held), pair, menu scrim ----
  await page.waitForSelector('.rating-panel, .pair-panel', { timeout: 10000 }).catch(() => {});
  await waitForStimulus(page);
  await snap(page, dir, report, ++i, 'trial-first');
  if (await page.locator('.rating-panel').count()) {
    await page.locator('#viewport').dispatchEvent('pointerdown');
    await page.waitForTimeout(250);
    await snap(page, dir, report, ++i, 'trial-revealing');
    await page.locator('#viewport').dispatchEvent('pointerup');
  }
  await page.locator('#menu').click({ timeout: 5000 }).catch(() => {});
  await page.waitForTimeout(200);
  await snap(page, dir, report, ++i, 'trial-menu');
  await page.locator('.scrim #continue').click({ timeout: 5000 }).catch(() => {});

  // Submit a few trials so a pair shows up eventually; screenshot one if seen.
  // Mirrors helpers.submitOneTrial: capture the trial id, click, wait for the
  // id to change — clicking mid-re-render detaches the button otherwise.
  let sawPair = false;
  for (let t = 0; t < 8 && !sawPair; t++) {
    await page.waitForSelector('.trial[data-trial-id]', { timeout: 10000 }).catch(() => {});
    const before = await page.locator('.trial').getAttribute('data-trial-id').catch(() => null);
    if (before == null) break;
    await page.waitForSelector('.rating-panel, .pair-panel', { timeout: 10000 }).catch(() => {});
    if (await page.locator('.pair-panel').count()) {
      sawPair = true;
      await waitForStimulus(page);
      await snap(page, dir, report, ++i, 'trial-pair');
      await page.locator('.pair-panel button[data-c="tie"]').click();
    } else if (await page.locator('.rating-panel').count()) {
      await page.locator('.rating-panel button[data-r="2"]').click();
    } else {
      break;
    }
    await page
      .waitForFunction(
        (old) => document.querySelector('.trial')?.getAttribute('data-trial-id') !== old,
        before,
        { timeout: 10000 },
      )
      .catch(() => {});
  }

  // ---- End session via menu ----
  await page.waitForSelector('.rating-panel, .pair-panel', { timeout: 10000 }).catch(() => {});
  await page.locator('#menu').click({ timeout: 5000 }).catch(() => {});
  await page.locator('.scrim #end').click({ timeout: 5000 }).catch(() => {});
  await page.waitForTimeout(200);
  await snap(page, dir, report, ++i, 'session-done');

  // ---- Suggest form ----
  await page.goto('/');
  await page.locator('.squintly-tabs button[data-tab="suggest"]').click({ timeout: 5000 }).catch(() => {});
  await page.waitForSelector('[data-screen="suggest"]', { timeout: 5000 }).catch(() => {});
  await snap(page, dir, report, ++i, 'suggest');

  // ---- Curator: stream, peek, settings, flag, curate, threshold ----
  await page.goto('/');
  await page.evaluate(() => {
    localStorage.setItem('squintly:curator_id', crypto.randomUUID());
  });
  await page.locator('.squintly-tabs button[data-tab="curator"]').click();
  await page.waitForSelector('[data-screen="stream"]', { timeout: 5000 });
  await page.waitForSelector('.curator-meta-row', { timeout: 5000 }).catch(() => {});
  await snap(page, dir, report, ++i, 'curator-stream');

  await page.locator('#settings').click({ timeout: 5000 }).catch(() => {});
  await page.waitForTimeout(200);
  await snap(page, dir, report, ++i, 'curator-settings');
  await page.locator('#cf-cancel').click({ timeout: 5000 }).catch(() => {});

  await page.locator('#take').click({ timeout: 5000 }).catch(() => {});
  await page.waitForSelector('[data-screen="curate"]', { timeout: 5000 }).catch(() => {});
  await snap(page, dir, report, ++i, 'curator-curate');

  // Toggle a group + a size chip, open the preview strip.
  await page.locator('.curator-group-btn[data-group="core_zensim"]').click({ timeout: 3000 }).catch(() => {});
  await page.locator('.curator-chip:not([disabled])').first().click({ timeout: 3000 }).catch(() => {});
  await page.locator('#preview-wrap summary').click({ timeout: 3000 }).catch(() => {});
  await page.waitForTimeout(400);
  await snap(page, dir, report, ++i, 'curator-curate-preview');

  await page.locator('#find-thr').click({ timeout: 5000 }).catch(() => {});
  await page.waitForSelector('[data-screen="threshold"]', { timeout: 5000 }).catch(() => {});
  await waitForThresholdPaint(page); // anchor pre-encode takes a few seconds
  await snap(page, dir, report, ++i, 'curator-threshold', { fullPage: false });

  const q = page.locator('#qslider');
  if (await q.count()) {
    await q.evaluate((el) => {
      (el as HTMLInputElement).value = '35';
      el.dispatchEvent(new Event('input', { bubbles: true }));
      el.dispatchEvent(new Event('change', { bubbles: true }));
    });
    await waitForThresholdPaint(page);
    await snap(page, dir, report, ++i, 'curator-threshold-q35', { fullPage: false });
  }

  await context.close();
  return report;
}

function renderMarkdown(reports: ViewportReport[]): string {
  const lines: string[] = [
    `# Squintly UX audit — ${new Date().toISOString()}`,
    '',
    `Base URL: ${BASE_URL}`,
    '',
  ];
  for (const r of reports) {
    const v = r.viewport;
    lines.push(`## ${v.name} (${v.width}×${v.height} @ ${v.deviceScaleFactor}x)`, '');
    for (const s of r.screens) {
      const flags: string[] = [];
      if (s.horizontalOverflow) flags.push(`**H-OVERFLOW ${s.scrollWidth}px > ${s.viewportWidth}px**`);
      if (s.interceptions.length) flags.push(`**${s.interceptions.length} intercepted tap(s)**`);
      lines.push(`- \`${s.file}\` ${flags.join(' · ') || 'ok'}`);
      for (const it of s.interceptions) {
        lines.push(`  - ${it.selector} "${it.text}" → hits ${it.hitDescription}`);
      }
      if (s.tinyTargets.length) {
        lines.push(`  - small targets: ${s.tinyTargets.join(', ')}`);
      }
    }
    lines.push('');
  }
  return lines.join('\n');
}

async function main(): Promise<void> {
  mkdirSync(OUT_ROOT, { recursive: true });
  // Load the curator fixture so the curator screens have candidates. The blob
  // base defaults to the e2e mock-coefficient port.
  const blobBase = process.env.AUDIT_BLOB_BASE ?? 'http://127.0.0.1:18181';
  try {
    const body = readFileSync(join(dirname(fileURLToPath(import.meta.url)), '../e2e/curator-fixture.jsonl'), 'utf-8');
    const r = await fetch(`${BASE_URL}/api/curator/manifest`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ kind: 'jsonl', body, blob_url_base: blobBase }),
    });
    console.log(`curator fixture load: ${r.status}`);
  } catch (e) {
    console.log(`curator fixture load skipped: ${(e as Error).message}`);
  }
  // Software canvas raster: accelerated 2D canvases screenshot as black.
  const browser = await chromium.launch({
    args: ['--disable-accelerated-2d-canvas', '--disable-gpu'],
  });
  const reports: ViewportReport[] = [];
  for (const vp of VIEWPORTS) {
    console.log(`auditing ${vp.name} (${vp.width}×${vp.height})…`);
    reports.push(await auditViewport(browser, vp));
  }
  await browser.close();
  writeFileSync(join(OUT_ROOT, 'audit-report.json'), JSON.stringify(reports, null, 2));
  writeFileSync(join(OUT_ROOT, 'REPORT.md'), renderMarkdown(reports));
  console.log(`\nwrote ${OUT_ROOT}/REPORT.md`);
  const bad = reports.flatMap((r) =>
    r.screens.filter((s) => s.horizontalOverflow || s.interceptions.length).map((s) => `${r.viewport.name}/${s.screen}`),
  );
  if (bad.length) {
    console.log(`FINDINGS on: ${bad.join(', ')}`);
    process.exitCode = 2;
  } else {
    console.log('no horizontal overflow or intercepted taps found');
  }
}

void main();
