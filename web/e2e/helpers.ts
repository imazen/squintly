// Small page-helpers shared across spec files.

import { expect, type Page, type APIRequestContext } from '@playwright/test';

export async function gotoFresh(page: Page) {
  // Wipe localStorage so each test starts with a clean observer/calibration.
  await page.context().clearCookies();
  await page.goto('/');
  await page.evaluate(() => localStorage.clear());
  await page.reload();
}

/// Enter as an operator, with the curator tab available.
///
/// Curator is no longer shipped to every participant — it is an operator tool
/// on an anonymous-participant origin, so it is opt-in per browser via the
/// `#curator` hash. This is that opt-in, and it is deliberately the same door
/// an operator walks through rather than a test-only backdoor.
export async function gotoFreshAsOperator(page: Page) {
  await gotoFresh(page);
  await page.goto('/#curator');
  await page.reload();
}

export async function clickBegin(page: Page) {
  await expect(page.getByRole('heading', { name: /Image Discrimination Study/i })).toBeVisible();
  await page.getByRole('button', { name: /^Begin$/ }).click();
}

export async function skipCalibration(page: Page) {
  // First stage: card resize. Skip lands at "Roughly how close…".
  await page.getByRole('button', { name: /^Skip$/ }).click();
  // Second stage isn't reached on skip path because we routed to onDone({nulls})
  // in the calibration helper; sometimes the screen has a "Skip" button there.
  // In either case the next visible heading is the profile form.
}

export async function completeProfileAndStart(page: Page) {
  await page.getByRole('button', { name: /^room$/ }).click();
  await page.getByRole('button', { name: /^no$/ }).click();
  await page.getByRole('button', { name: /^25-35$/ }).click();
  await page.getByRole('button', { name: /Start rating/i }).click();
}

/**
 * Look at every arm the trial requires, so the answer panel unlocks.
 *
 * The panel is gated until both A and B have been on screen — "which is closer
 * to the original" is not answerable from one of them. A real observer has to
 * do this, so the helpers do too rather than reaching past the gate.
 */
export async function satisfyGate(page: Page) {
  await page.waitForSelector('.viewport.all-ready', { timeout: 20_000 }).catch(() => {});
  for (const v of ['a', 'b'] as const) {
    const btn = page.locator(`.view-switch button[data-view="${v}"]`);
    if (await btn.count()) await btn.click();
  }
  await expect(page.locator('#panel')).toHaveAttribute('data-gated', 'no', { timeout: 15_000 });
}

/** Submit one single-stimulus trial with the given 4-tier rating. */
export async function rateSingle(page: Page, rating: 1 | 2 | 3 | 4) {
  await page.waitForSelector('.rating-panel', { state: 'visible', timeout: 10_000 });
  await satisfyGate(page);
  await page.locator(`.rating-panel button[data-r="${rating}"]`).click();
}

/** Submit one pair trial with A/tie/B. */
export async function ratePair(page: Page, choice: 'a' | 'tie' | 'b') {
  await page.waitForSelector('.pair-panel', { state: 'visible', timeout: 10_000 });
  await satisfyGate(page);
  await page.locator(`.pair-panel button[data-c="${choice}"]`).click();
}

/** Wait for whichever trial type rendered. */
export async function awaitAnyTrialPanel(page: Page) {
  await page.waitForSelector('.rating-panel, .pair-panel', { state: 'visible', timeout: 10_000 });
}

/**
 * Atomic "submit one trial" — waits for whichever panel mounted, captures the
 * current trial-id so we can detect the next-trial render, clicks, then waits
 * for the trial container's data-trial-id to change. Eliminates the race where
 * the previous trial's panel is briefly still in the DOM after click.
 */
export async function submitOneTrial(
  page: Page,
  opts: { single?: 1 | 2 | 3 | 4; pair?: 'a' | 'tie' | 'b' } = {},
) {
  await page.waitForSelector('.trial[data-trial-id]', { state: 'visible', timeout: 10_000 });
  const before = await page.locator('.trial').getAttribute('data-trial-id');
  await page.waitForSelector('.rating-panel, .pair-panel', { state: 'visible', timeout: 10_000 });
  await satisfyGate(page);
  const kind = await page.evaluate(() => {
    if (document.querySelector('.rating-panel')) return 'single';
    if (document.querySelector('.pair-panel')) return 'pair';
    return null;
  });
  if (kind === 'single') {
    const r = opts.single ?? 2;
    await page.locator(`.rating-panel button[data-r="${r}"]`).click();
  } else if (kind === 'pair') {
    const c = opts.pair ?? 'a';
    await page.locator(`.pair-panel button[data-c="${c}"]`).click();
  } else {
    throw new Error('no trial panel mounted');
  }
  // Wait for the next trial to mount before returning.
  await page.waitForFunction(
    (old) => {
      const el = document.querySelector('.trial');
      const cur = el?.getAttribute('data-trial-id');
      return cur && cur !== old;
    },
    before,
    { timeout: 10_000 },
  );
}

export async function statsOf(api: APIRequestContext) {
  const r = await api.get('/api/stats');
  expect(r.ok()).toBeTruthy();
  return r.json();
}

/// The shared admin token, matching `global-setup`. Used for API-level calls
/// where driving a browser sign-in would be noise.
export const ADMIN_TOKEN = 'e2e-admin-token';

/// Sign the browser in as an admin, through the real magic-link flow.
///
/// No backdoor: this requests a link exactly as a person would, reads it out of
/// the mock mail sink, and follows it. So it also serves as end-to-end coverage
/// that admin sign-in actually works.
export async function signInAsAdmin(page: Page, coefficientPort: number) {
  const start = await page.request.post('/api/auth/start', {
    data: {
      email: 'admin@e2e.test',
      observer_id: null,
      origin: page.url().startsWith('http') ? new URL(page.url()).origin : undefined,
    },
  });
  if (!start.ok()) throw new Error(`auth/start ${start.status()}: ${await start.text()}`);

  const outbox = (await (
    await page.request.get(`http://127.0.0.1:${coefficientPort}/outbox`)
  ).json()) as Array<{ TextBody?: string }>;
  const last = outbox[outbox.length - 1];
  const token = last?.TextBody?.split('token=')[1]?.split(/\s/)[0];
  if (!token) throw new Error('no magic link in the outbox');

  await page.goto(`/api/auth/verify?token=${token}`);
  const who = await (await page.request.get('/api/auth/whoami')).json();
  if (!who.is_admin) throw new Error(`signed in but not admin: ${JSON.stringify(who)}`);
}
