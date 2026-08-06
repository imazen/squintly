import { expect, test } from './fixtures';

import {
  clickBegin,
  completeProfileAndStart,
  gotoLanding,
  submitOneTrial,
} from './helpers';

test.describe('the front page', () => {
  // Opening squintly used to run straight into onboarding, and for a returning
  // observer straight into the next trial — so the decision to take part was
  // made on the one screen with nothing on it to decide from.
  test('explains the task and offers a way in without starting one', async ({ page }) => {
    await gotoLanding(page);
    await expect(page.locator('[data-screen="landing"]')).toBeVisible();
    await expect(page.locator('.trial[data-trial-id]')).toHaveCount(0);

    // What the judgement actually is, not just that there is one.
    await expect(page.locator('.lede')).toContainText(/closer to the original/i);
    await expect(page.locator('#landing-start')).toBeVisible();
  });

  // Both doors, and neither dressed as the lesser one: an anonymous observer's
  // data is worth exactly as much as a signed-in one's.
  test('offers guest and sign-in as equal-weight buttons', async ({ page }) => {
    await gotoLanding(page);
    await expect(page.locator('#landing-start')).toContainText(/guest/i);
    const signin = page.locator('#landing-signin');
    await expect(signin).toBeVisible();
    await expect(page.locator('[data-screen="landing"]')).toContainText(/another device/i);

    // Sign-in was 0.85rem underlined text — not a button, and not findable by
    // someone scanning for one. Both ways in are the same size, because neither
    // is the lesser option.
    const [start, sign] = await Promise.all([
      page.locator('#landing-start').boundingBox(),
      signin.boundingBox(),
    ]);
    // Same type size — not equal height, which differs legitimately when one
    // label wraps and the other does not on a narrow screen.
    const size = (l: typeof signin) =>
      l.evaluate((el) => ({
        fs: parseFloat(getComputedStyle(el).fontSize),
        pad: getComputedStyle(el).padding,
      }));
    const [a, b] = await Promise.all([size(page.locator('#landing-start')), size(signin)]);
    expect(b.fs, 'sign-in must not be fine print').toBeGreaterThan(15);
    expect(b.fs, 'both ways in are the same size').toBeCloseTo(a.fs, 1);
    expect(b.pad).toBe(a.pad);
    // Stacked on a narrow screen, they share the width; side by side, neither
    // is squeezed to a fraction of the other.
    expect(Math.min(sign!.width, start!.width) / Math.max(sign!.width, start!.width)).toBeGreaterThan(
      0.5,
    );
  });

  // The page was set in fine print throughout — a 0.72rem legend under a
  // 0.72rem bar is unreadable at arm's length on a phone.
  test('nothing on the front page is fine print', async ({ page }) => {
    await gotoLanding(page);
    const smallest = await page.evaluate(() => {
      const els = [...document.querySelectorAll<HTMLElement>('[data-screen="landing"] *')];
      return els
        .filter((e) => (e.textContent ?? '').trim().length > 12 && e.offsetParent !== null)
        .reduce((m, e) => Math.min(m, parseFloat(getComputedStyle(e).fontSize)), 99);
    });
    expect(smallest, 'no body text below 13px').toBeGreaterThanOrEqual(13);
  });

  // A raw response count says nothing without the target beside it — 400
  // ratings is most of the way for one study and a rounding error for another.
  test('shows each study against its own targets', async ({ page }) => {
    await gotoLanding(page);
    const goals = page.locator('.goal');
    expect(await goals.count()).toBeGreaterThan(0);
    const first = goals.first();
    await expect(first.locator('.goal-name')).not.toBeEmpty();
    await expect(first.locator('.goal-bar')).toBeVisible();
    // The legend names a threshold, so the bar cannot be an unlabelled sliver.
    await expect(first.locator('.goal-legend')).toContainText(/\d/);
  });

  // A board you cannot find yourself on is a scoreboard for other people.
  test('marks your own row once you have rated', async ({ page }) => {
    await gotoLanding(page);
    await page.locator('#landing-start').click();
    await clickBegin(page);
    await page.getByRole('button', { name: /^Skip$/ }).click();
    await completeProfileAndStart(page);
    await submitOneTrial(page);

    // The board moved off the front page to its own route: it made the front
    // page long enough that the decision to take part competed with a table
    // nobody needs before rating anything, and a board is something people come
    // back to, so it wants a URL. The front page keeps a count and a way in.
    await page.goto('/');
    await expect(page.locator('[data-screen="landing"]')).toBeVisible();
    await expect(page.locator('.board tr')).toHaveCount(0);
    await page.locator('#landing-board').click();

    await expect(page.locator('[data-screen="board"]')).toBeVisible();
    const me = page.locator('.board tr.me');
    await expect(me).toHaveCount(1);
    await expect(me.locator('.you')).toBeVisible();
    // Reachable by URL, which is the point of it being a route.
    await expect(page).toHaveURL(/\/board$/);
  });

  test('the front page is where a session ends up, not the next trial', async ({ page }) => {
    await gotoLanding(page);
    await page.locator('#landing-start').click();
    await clickBegin(page);
    await page.getByRole('button', { name: /^Skip$/ }).click();
    await completeProfileAndStart(page);
    await page.waitForSelector('.trial[data-trial-id]', { timeout: 30_000 });

    await page.goto('/');
    await expect(page.locator('[data-screen="landing"]')).toBeVisible();
  });
});

test.describe('the JXL flag instruction', () => {
  // Headless Chromium does not decode JXL, so the panel is expected to render.
  // If a future Playwright ships JXL support this test starts skipping itself,
  // which is why it asserts the *supported* branch too rather than assuming.
  test('tells a Chromium user how to turn JPEG XL on', async ({ page }) => {
    await gotoLanding(page);
    const panel = page.locator('.jxl-advice');
    await expect(panel).toBeVisible({ timeout: 15_000 });
    await expect(panel).toContainText(/JPEG XL/i);

    // The address must be present and copyable. It cannot be a link — browsers
    // refuse navigation to chrome:// from a page — so a copy button is the only
    // followable form, and its absence would leave the instruction unusable.
    const flag = page.locator('#jxl-flag');
    await expect(flag).toHaveText(/^(chrome|edge):\/\/flags\/#enable-jxl-image-format$/);
    await expect(page.locator('#jxl-copy')).toBeVisible();
  });

  test('does not block the page from being used', async ({ page }) => {
    // Advice, never a gate. The probe decodes a test image per format, so it
    // must not sit between somebody and the button they came to press.
    await gotoLanding(page);
    await expect(page.locator('#landing-start')).toBeEnabled();
    await page.locator('#landing-start').click();
    await expect(page.locator('[data-screen="landing"]')).toBeHidden({ timeout: 15_000 });
  });
});
