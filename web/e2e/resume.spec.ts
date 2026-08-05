import { expect, test } from './fixtures';
import { type Page } from '@playwright/test';

import { clickBegin, completeProfileAndStart, gotoFresh, passInstructions } from './helpers';

/// Walk a fresh visitor all the way through onboarding into trials.
async function onboard(page: Page) {
  await gotoFresh(page);
  await clickBegin(page);
  await page.getByRole('button', { name: /^Skip$/ }).click();
  await completeProfileAndStart(page);
  await page.waitForSelector('.trial[data-trial-id]', { timeout: 30_000 });
}

test.describe('reopening the app', () => {
  // Reopening used to drop a returning observer back on the welcome screen and
  // walk them through Begin -> calibration -> profile again, even though the
  // observer id, profile and calibration were all already stored — three
  // screens of friction in front of the thing they came back to do.
  //
  // The property is "no interaction required", not "a particular screen is
  // visible": resume is fast enough that the interstitial can come and go
  // between polls, so asserting on it would be timing-dependent.
  test('a returning observer lands in trials with no interaction', async ({ page }) => {
    await onboard(page);

    await page.goto('/');

    await passInstructions(page);
    // Deliberately no clicks.
    await page.waitForSelector('.trial[data-trial-id]', { timeout: 30_000 });
    await expect(page.locator('.rating-panel, .pair-panel')).toBeVisible();
    await expect(
      page.getByRole('heading', { name: /Image Discrimination Study/i }),
    ).toHaveCount(0);
  });

  test('a first-time visitor still sees the welcome screen', async ({ page }) => {
    await gotoFresh(page);
    await expect(page.getByRole('heading', { name: /Image Discrimination Study/i })).toBeVisible();
    await expect(page.locator('.trial[data-trial-id]')).toHaveCount(0);
  });

  // Resuming must not be a trap: there has to be a way back. Hold the session
  // request so the interstitial stays up long enough to click, rather than
  // racing it.
  test('a returning observer can decline and start from the beginning', async ({ page }) => {
    await onboard(page);

    let release: () => void = () => {};
    const gate = new Promise<void>((r) => (release = r));
    await page.route('**/api/session', async (route) => {
      await gate;
      await route.continue();
    });

    await page.goto('/');

    await passInstructions(page);
    await page.locator('#not-now').click();
    await expect(page.getByRole('heading', { name: /Image Discrimination Study/i })).toBeVisible();
    release();
  });
});

test.describe('calibration stickiness', () => {
  // The card measurement was persisted, but the screen reopened at a fixed
  // slider value and its Skip returned nulls that the caller wrote straight
  // over the stored value — so re-entering just to check could destroy it.
  test('a stored measurement is reloaded, and Skip preserves it', async ({ page }) => {
    await gotoFresh(page);
    await clickBegin(page);

    const slider = page.locator('#slider');
    await slider.fill('320');
    await slider.dispatchEvent('input');
    await page.getByRole('button', { name: /Looks right/i }).click();
    // Stage 2 is the distance step; skip it — stage 1's value must survive.
    await page.getByRole('button', { name: /^Skip$/ }).click();

    const stored = () =>
      page.evaluate(() => JSON.parse(localStorage.getItem('squintly:calibration') || 'null'));
    await expect.poll(async () => (await stored())?.css_px_per_mm ?? 0).toBeGreaterThan(0);
    const first = await stored();

    // Back to the welcome screen (this observer has no profile yet, so a plain
    // reload lands there) and reopen calibration from the link.
    await page.goto('/');
    await passInstructions(page);
    await page.locator('#calibrate-link').click();
    await expect(page.locator('#slider')).toBeVisible();

    // It must reopen where it was left, not at a default.
    const value = Number(await page.locator('#slider').inputValue());
    expect(Math.abs(value - 320), `slider reopened at ${value}, expected ~320`).toBeLessThan(2);

    await page.getByRole('button', { name: /^Skip$/ }).click();
    const after = await stored();
    expect(after?.css_px_per_mm, 'Skip must preserve, not erase').toBeCloseTo(
      first.css_px_per_mm,
      3,
    );
  });

  // Calibrate was a permanent tab; it is a one-off measurement the app
  // remembers, so it lives behind a link that says whether it is already done.
  test('calibration is a link, not a tab', async ({ page }) => {
    await gotoFresh(page);
    await expect(page.locator('.squintly-tabs button[data-tab="calibrate"]')).toHaveCount(0);
    await expect(page.locator('#calibrate-link')).toContainText(/calibrate screen size/i);
  });
});

test.describe('the pause menu', () => {
  async function openMenu(page: Page) {
    await onboard(page);
    await page.locator('#menu').click();
    await expect(page.getByRole('heading', { name: /^Pause$/ })).toBeVisible();
    // The study list is fetched, so the select starts with a valueless
    // "loading…" placeholder. Reading it before the fetch lands finds no real
    // options at all — which passes in isolation and fails under suite load.
    await expect
      .poll(async () => page.locator('#menu-study option[value]').count(), { timeout: 15_000 })
      .toBeGreaterThan(1);
  }

  // It used to offer only continue/end, so changing anything mid-session meant
  // abandoning it and hunting the welcome screen.
  test('offers study, comparison mode, calibration and shortcuts', async ({ page }) => {
    await openMenu(page);
    await expect(page.locator('#menu-study')).toBeVisible();
    await expect(page.locator('#menu-mode')).toBeVisible();
    await expect(page.locator('#menu-calibrate')).toBeVisible();
    await expect(page.locator('#menu-keys')).toBeVisible();
    // Every listed study is reachable from here.
    expect(await page.locator('#menu-study option[value]').count()).toBeGreaterThan(1);
  });

  // A session belongs to exactly one study, so switching cannot be applied in
  // place — its trials would end up filed under a study the observer left.
  test('switching study starts a fresh session on that study', async ({ page }) => {
    await openMenu(page);
    const before = await page.evaluate(() => localStorage.getItem('squintly:study_id'));
    const options = await page.locator('#menu-study option[value]').all();
    let target: string | null = null;
    for (const o of options) {
      const v = await o.getAttribute('value');
      if (v && v !== before) {
        target = v;
        break;
      }
    }
    expect(target, 'needed a second study to switch to').toBeTruthy();

    await page.locator('#menu-study').selectOption(target!);
    // Back in trials, on the new study, without visiting the welcome screen.
    await page.waitForSelector('.trial[data-trial-id]', { timeout: 30_000 });
    await expect(page.getByRole('heading', { name: /Image Discrimination Study/i })).toHaveCount(0);
    expect(await page.evaluate(() => localStorage.getItem('squintly:study_id'))).toBe(target);
  });

  test('changing comparison mode applies without leaving the trial', async ({ page }) => {
    await openMenu(page);
    const current = await page.evaluate(
      () => document.querySelector<HTMLElement>('.trial')!.dataset.inputMode,
    );
    const options = (await page.locator('#menu-mode option').all()).length;
    test.skip(options < 2, 'this device offers only one mode');
    let other: string | null = null;
    for (const o of await page.locator('#menu-mode option').all()) {
      const v = await o.getAttribute('value');
      if (v && v !== current) {
        other = v;
        break;
      }
    }
    await page.locator('#menu-mode').selectOption(other!);
    await page.waitForSelector(`.trial[data-input-mode="${other}"]`, { timeout: 15_000 });
    // And it sticks.
    expect(await page.evaluate(() => localStorage.getItem('squintly_input_mode'))).toBe(other);
  });

  test('re-measuring the screen returns to trials, not to the welcome screen', async ({ page }) => {
    await openMenu(page);
    await page.locator('#menu-calibrate').click();
    await expect(page.locator('#slider')).toBeVisible();
    await page.getByRole('button', { name: /Looks right/i }).click();
    await page.getByRole('button', { name: /^Skip$/ }).click();
    await page.waitForSelector('.trial[data-trial-id]', { timeout: 30_000 });
    await expect(page.getByRole('heading', { name: /Image Discrimination Study/i })).toHaveCount(0);
  });

  test('keep going dismisses without disturbing the trial', async ({ page }) => {
    await openMenu(page);
    const id = await page.locator('.trial').getAttribute('data-trial-id');
    await page.locator('#continue').click();
    await expect(page.locator('.scrim')).toHaveCount(0);
    expect(await page.locator('.trial').getAttribute('data-trial-id')).toBe(id);
  });
});

test.describe('the reviewer leaderboard', () => {
  // The endpoint existed but nothing linked to it, so the board was
  // unreachable from the app it was built for.
  test('is reachable from the pause menu and shows reliability beside volume', async ({
    page,
  }) => {
    await onboard(page);
    await page.locator('#menu').click();
    await expect(page.getByRole('heading', { name: /^Pause$/ })).toBeVisible();
    await page.locator('#menu-leaderboard').click();

    const body = page.locator('#menu-body');
    await expect(body).toBeVisible();
    // Either a board or an honest empty state — never a silent no-op. Polling
    // on length alone passed instantly on the "Loading…" placeholder, so wait
    // for it to be replaced rather than for it to be non-empty.
    await expect
      .poll(async () => body.innerText(), { timeout: 15_000 })
      .not.toMatch(/Loading the board/i);
    const text = await body.innerText();
    if (text.match(/No reviewers/i)) return;

    // Volume and reliability together: a board that ranks on count alone
    // rewards the behaviour the attention checks exist to catch.
    for (const col of ['Reviewer', 'Trials', 'Self-agree', 'Checks']) {
      expect(text, `missing column ${col}`).toContain(col);
    }
  });
});
