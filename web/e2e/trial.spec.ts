import { expect, test } from '@playwright/test';

import {
  awaitAnyTrialPanel,
  clickBegin,
  completeProfileAndStart,
  gotoFresh,
  ratePair,
  rateSingle,
  submitOneTrial,
} from './helpers';

test.describe('trial loop', () => {
  test('records a rating and advances to the next trial', async ({ page, request }) => {
    const before = await (await request.get('/api/stats')).json();
    await gotoFresh(page);
    await clickBegin(page);
    await page.getByRole('button', { name: /^Skip$/ }).click();
    await completeProfileAndStart(page);

    await submitOneTrial(page);
    // The next trial should mount within a couple of seconds.
    await awaitAnyTrialPanel(page);

    const after = await (await request.get('/api/stats')).json();
    expect(after.responses).toBeGreaterThan(before.responses);
    expect(after.sessions).toBeGreaterThan(before.sessions);
  });

  test('hold-to-reveal swaps to reference image on single-stimulus trials', async ({ page }) => {
    await gotoFresh(page);
    await clickBegin(page);
    await page.getByRole('button', { name: /^Skip$/ }).click();
    await completeProfileAndStart(page);
    await awaitAnyTrialPanel(page);

    // Trial type is sampler-chosen, so the first one may be a pair. Advance
    // until a single-stimulus trial appears rather than skipping the test:
    // a runtime self-skip is a silent pass that asserts nothing (global
    // CLAUDE.md, "NO GRACEFUL SKIPS"). If none shows up in 15 trials the
    // sampler is broken and this must fail loudly.
    let found = false;
    for (let i = 0; i < 15; i++) {
      if (await page.locator('.rating-panel').isVisible()) {
        found = true;
        break;
      }
      await submitOneTrial(page, { pair: 'tie' });
      await awaitAnyTrialPanel(page);
    }
    expect(found, 'no single-stimulus trial served in 15 trials').toBe(true);

    const img = page.locator('#stimulus');
    const initialSrc = await img.getAttribute('src');
    // Real pointer events, not dispatchEvent: the handler reads pointerId and
    // calls setPointerCapture, neither of which a synthetic Event provides.
    const box = (await page.locator('#viewport').boundingBox())!;
    const cx = box.x + box.width / 2;
    const cy = box.y + box.height / 2;
    await page.mouse.move(cx, cy);
    await page.mouse.down();
    // While held the src should switch to the reference.
    await expect.poll(async () => img.getAttribute('src')).not.toBe(initialSrc);
    await page.mouse.up();
    await expect.poll(async () => img.getAttribute('src')).toBe(initialSrc);
  });

  test('rating ten trials awards the first_10 milestone badge', async ({ page, request }) => {
    await gotoFresh(page);
    await clickBegin(page);
    await page.getByRole('button', { name: /^Skip$/ }).click();
    await completeProfileAndStart(page);

    for (let i = 0; i < 10; i++) {
      await submitOneTrial(page);
    }
    // Pull the observer id straight from localStorage, then GET the profile.
    const observerId = await page.evaluate(() => localStorage.getItem('squintly:observer_id'));
    expect(observerId).not.toBeNull();
    const profile = await (await request.get(`/api/observer/${observerId}/profile`)).json();
    expect(profile.total_trials).toBeGreaterThanOrEqual(10);
    const slugs = (profile.badges as Array<{ slug: string }>).map((b) => b.slug);
    expect(slugs).toContain('first_10');
  });

  /// The stimulus must render at exactly 1:1 device pixels, never smaller.
  ///
  /// A display downscale means the observer rates the *browser's* resample of
  /// the encode instead of the encode — the artefacts under test get averaged
  /// away, worst exactly where the study cares most (high-DPR phones, large
  /// sources). Anything larger than the viewport is panned, not shrunk.
  test('stimulus renders at exactly 1:1 device pixels and pans when oversized', async ({ page }) => {
    await gotoFresh(page);
    await clickBegin(page);
    await page.getByRole('button', { name: /^Skip$/ }).click();
    await completeProfileAndStart(page);

    for (let i = 0; i < 6; i++) {
      await page.waitForSelector('.trial[data-trial-id]', { timeout: 10_000 });
      await page
        .waitForFunction(
          () => {
            const im = document.querySelector<HTMLImageElement>('#stimulus');
            return !!im && im.complete && im.naturalWidth > 0 && im.style.width !== '';
          },
          undefined,
          { timeout: 10_000 },
        )
        .catch(() => {});

      const m = await page.evaluate(() => {
        const im = document.querySelector<HTMLImageElement>('#stimulus')!;
        const vp = document.querySelector<HTMLElement>('#viewport')!;
        const r = im.getBoundingClientRect();
        const v = vp.getBoundingClientRect();
        return {
          ratio: im.naturalWidth / (r.width * window.devicePixelRatio),
          overflowsX: r.width > v.width + 1,
          // Panning must be possible whenever the stimulus overflows.
          hint: document.querySelector('#hint')!.textContent ?? '',
        };
      });

      expect(
        Math.abs(m.ratio - 1),
        'stimulus is not at 1:1 device pixels — a downscale invalidates the rating',
      ).toBeLessThan(0.02);
      if (m.overflowsX) {
        expect(m.hint, 'an oversized stimulus must advertise panning').toContain('drag');
      }

      await submitOneTrial(page);
    }
  });

  /// Panning must survive the encoded<->reference swap, or the observer is
  /// comparing two different parts of the picture.
  test('pan offset is preserved when swapping to the reference', async ({ page }) => {
    await gotoFresh(page);
    await clickBegin(page);
    await page.getByRole('button', { name: /^Skip$/ }).click();
    await completeProfileAndStart(page);

    // Find a single-stimulus trial whose image overflows the viewport.
    let found = false;
    for (let i = 0; i < 20 && !found; i++) {
      await page.waitForSelector('.trial[data-trial-id]', { timeout: 10_000 });
      await page
        .waitForFunction(
          () => {
            const im = document.querySelector<HTMLImageElement>('#stimulus');
            return !!im && im.complete && im.style.width !== '';
          },
          undefined,
          { timeout: 10_000 },
        )
        .catch(() => {});
      found = await page.evaluate(() => {
        const im = document.querySelector<HTMLImageElement>('#stimulus')!;
        const vp = document.querySelector<HTMLElement>('#viewport')!;
        return (
          !!document.querySelector('.rating-panel') &&
          im.getBoundingClientRect().width > vp.getBoundingClientRect().width + 20
        );
      });
      if (!found) await submitOneTrial(page);
    }
    expect(found, 'no oversized single-stimulus trial appeared in 20 trials').toBe(true);

    const box = (await page.locator('#viewport').boundingBox())!;
    const cx = box.x + box.width / 2;
    const cy = box.y + box.height / 2;
    await page.mouse.move(cx, cy);
    await page.mouse.down();
    for (let s = 1; s <= 5; s++) await page.mouse.move(cx + s * 16, cy);
    const held = await page.evaluate(() => ({
      transform: document.querySelector<HTMLImageElement>('#stimulus')!.style.transform,
      showingReference: !!document.querySelector('.trial.revealing'),
    }));
    await page.mouse.up();
    const released = await page.evaluate(
      () => document.querySelector<HTMLImageElement>('#stimulus')!.style.transform,
    );

    expect(held.showingReference, 'press-and-hold should reveal the reference').toBe(true);
    expect(held.transform).not.toBe('translate(0px, 0px)');
    expect(released, 'pan reset on release — the two views show different regions').toBe(
      held.transform,
    );
  });

  /// A pair trial asks which encode is "closer to original". For that question
  /// to mean anything the observer has to be able to SEE the original — and
  /// for a while they could not: `startReveal` was gated behind `!isPair` and
  /// nothing else reached the reference, which quietly turned a reference
  /// comparison into a preference test.
  test('pair trials can show the reference, and A/B/Original are distinct', async ({ page }) => {
    await gotoFresh(page);
    // The forced-choice study guarantees a pair trial.
    await page.locator('.study-option[data-study="ssim2-nonphoto"]').click();
    await clickBegin(page);
    await page.getByRole('button', { name: /^Skip$/ }).click();
    await completeProfileAndStart(page);
    await page.waitForSelector('.pair-panel', { timeout: 15_000 });

    const settle = () =>
      page.waitForFunction(
        () => {
          const i = document.querySelector<HTMLImageElement>('#stimulus');
          return !!i && i.complete && i.naturalWidth > 0;
        },
        undefined,
        { timeout: 15_000 },
      );
    const srcNow = async () => {
      await settle();
      return page.evaluate(() => document.querySelector<HTMLImageElement>('#stimulus')!.src);
    };

    await expect(page.locator('.view-switch button[data-view="ref"]')).toBeVisible();
    const a = await srcNow();
    await page.locator('.view-switch button[data-view="b"]').click();
    const b = await srcNow();
    await page.locator('.view-switch button[data-view="ref"]').click();
    const ref = await srcNow();

    expect(a, 'A and B must be different encodings').not.toBe(b);
    expect(ref, 'the reference must differ from A').not.toBe(a);
    expect(ref, 'the reference must differ from B').not.toBe(b);
    expect(ref, 'the reference should be the source proxy').toContain('/api/proxy/source/');
  });

  /// Magnification is integer-only and nearest-neighbour, and never goes below
  /// 1:1. A fractional factor would size some source pixels 2 device px and
  /// others 3 — fabricated structure in a study about which structure is real.
  test('zoom magnifies by exact integers, never below 1:1', async ({ page }) => {
    await gotoFresh(page);
    await clickBegin(page);
    await page.getByRole('button', { name: /^Skip$/ }).click();
    await completeProfileAndStart(page);
    await page.waitForSelector('.rating-panel, .pair-panel', { timeout: 15_000 });
    await page
      .waitForFunction(
        () => {
          const i = document.querySelector<HTMLImageElement>('#stimulus');
          return !!i && i.complete && i.naturalWidth > 0 && i.style.width !== '';
        },
        undefined,
        { timeout: 15_000 },
      )
      .catch(() => {});

    const measure = () =>
      page.evaluate(() => {
        const i = document.querySelector<HTMLImageElement>('#stimulus')!;
        return {
          devicePxPerImagePx:
            (i.getBoundingClientRect().width * window.devicePixelRatio) / i.naturalWidth,
          rendering: getComputedStyle(i).imageRendering,
        };
      });

    for (const z of [1, 2, 4]) {
      await page.locator(`.zoom-switch button[data-zoom="${z}"]`).click();
      await page.waitForTimeout(150);
      const m = await measure();
      expect(
        Math.abs(m.devicePxPerImagePx - z),
        `at ${z}x each image pixel must cover exactly ${z} device px`,
      ).toBeLessThan(0.02);
      expect(m.devicePxPerImagePx, 'never below 1:1').toBeGreaterThanOrEqual(0.98);
      expect(m.rendering, 'must be nearest-neighbour, not interpolated').toBe('pixelated');
    }
  });

  /// The A/B indicator was small muted text in a hint pill — the only cue for
  /// which stimulus you were looking at, in a task entirely about telling them
  /// apart. It is a segmented control with a thumb-sized target now.
  test('the active view is labelled prominently', async ({ page }) => {
    await gotoFresh(page);
    await clickBegin(page);
    await page.getByRole('button', { name: /^Skip$/ }).click();
    await completeProfileAndStart(page);
    await page.waitForSelector('.rating-panel, .pair-panel', { timeout: 15_000 });

    const active = page.locator('.view-switch button.on');
    await expect(active).toHaveCount(1);
    const box = (await active.boundingBox())!;
    expect(box.height, 'the view label must be a thumb-sized target').toBeGreaterThanOrEqual(40);
    const size = await active.evaluate((el) => parseFloat(getComputedStyle(el).fontSize));
    expect(size, 'the active-view label must not be tiny').toBeGreaterThanOrEqual(14);
  });
});
