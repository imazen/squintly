import { expect, test } from '@playwright/test';

import { clickBegin, completeProfileAndStart, gotoFresh } from './helpers';

/// Get onto a trial screen with the images decoded.
async function toTrial(page: import('@playwright/test').Page) {
  await gotoFresh(page);
  await clickBegin(page);
  await page.getByRole('button', { name: /^Skip$/ }).click();
  await completeProfileAndStart(page);
  await page.waitForSelector('.trial[data-trial-id]');
  await page.waitForSelector('.viewport:not(.is-loading)');
}

/// Answer the current trial and wait until the *next* one is fully up.
///
/// Waiting on `.viewport:not(.is-loading)` alone is a race: right after the
/// click the outgoing trial is still mounted and already not-loading, so the
/// wait returns instantly and the caller inspects the trial it just answered.
/// The trial id changing is the only reliable edge.
async function advance(page: import('@playwright/test').Page) {
  const before = await page.locator('.trial').getAttribute('data-trial-id');
  await page.locator('.rating-panel button, .pair-panel button').first().click();
  await expect
    .poll(async () => page.locator('.trial').getAttribute('data-trial-id'), { timeout: 15_000 })
    .not.toBe(before);
  await page.waitForSelector('.viewport:not(.is-loading)');
}

/// Advance until a trial of the wanted kind is on screen.
async function toKind(page: import('@playwright/test').Page, kind: 'single' | 'pair') {
  const sel = kind === 'pair' ? '.pair-panel' : '.rating-panel';
  for (let i = 0; i < 60; i++) {
    if (await page.locator(sel).count()) return true;
    await advance(page);
  }
  return false;
}

test.describe('trial input', () => {
  // The point of preloading: every variant is decoded before the observer can
  // answer, so flicking between them is a paint and not a fetch. If a variant
  // were still loading, comparing A to B would mean holding one of them in
  // memory across a network round trip.
  test('every variant is preloaded, so switching is instant', async ({ page }) => {
    await toTrial(page);
    await page.waitForSelector('.viewport.all-ready');

    const layers = await page.evaluate(() =>
      [...document.querySelectorAll<HTMLImageElement>('.viewport img.layer')].map((im) => ({
        layer: im.dataset.layer,
        complete: im.complete,
        decoded: im.naturalWidth > 0,
        shown: im.classList.contains('shown'),
        src: im.src,
      })),
    );

    expect(layers.length).toBeGreaterThanOrEqual(2);
    for (const l of layers) {
      expect(l.decoded, `layer ${l.layer} must be decoded before answering`).toBe(true);
    }
    // Exactly one visible, and `#stimulus` follows it — conditions capture and
    // grading geometry both read that id as "what the observer is looking at".
    expect(layers.filter((l) => l.shown)).toHaveLength(1);
    const stim = await page.evaluate(() => ({
      layer: document.querySelector<HTMLImageElement>('#stimulus')?.dataset.layer,
      count: document.querySelectorAll('#stimulus').length,
    }));
    expect(stim.count).toBe(1);
    expect(stim.layer).toBe(layers.find((l) => l.shown)!.layer);

    // Distinct sources — a stack of the same picture would compare nothing.
    const srcs = new Set(layers.map((l) => l.src));
    expect(srcs.size).toBe(layers.length);
  });

  // Answering before the judged image is painted would record a judgement of
  // something never seen.
  test('the response panel is disabled until the image is painted', async ({ page }) => {
    await gotoFresh(page);
    await clickBegin(page);
    await page.getByRole('button', { name: /^Skip$/ }).click();
    // Hold the images so the loading state is observable rather than a flash.
    let release: () => void = () => {};
    const gate = new Promise<void>((r) => (release = r));
    await page.route('**/api/proxy/source/**', async (route) => {
      await gate;
      await route.continue();
    });
    await completeProfileAndStart(page);
    await page.waitForSelector('.trial[data-trial-id]');

    // The reference is held on the wire, so the trial cannot be complete —
    // asserted unconditionally. This used to be wrapped in `if (loading)`,
    // which made it vacuous: the route pattern matched nothing (references are
    // served from /api/proxy/source/, not /api/sources/), so the gate never
    // applied and the branch never ran.
    const state = await page.evaluate(() => {
      const btn = document.querySelector<HTMLButtonElement>(
        '.rating-panel button, .pair-panel button',
      );
      const vp = document.querySelector('.viewport');
      return { disabled: btn?.disabled ?? null, loading: vp?.classList.contains('is-loading') };
    });
    expect(state.loading, 'a held reference means the trial is still loading').toBe(true);
    expect(state.disabled, 'cannot answer while a variant is still loading').toBe(true);
    await expect(page.locator('.viewport-status .spinner')).toBeVisible();

    release();
    await page.waitForSelector('.viewport:not(.is-loading)');
    await expect(
      page.locator('.rating-panel button, .pair-panel button').first(),
    ).toBeEnabled();
  });

  // Arrows are HELD, not tapped. They used to step a carousel, which meant the
  // keyboard and the mouse disagreed about what "left" does — the mouse held a
  // view down while the keyboard latched one.
  test('arrows and space hold rather than latch', async ({ page }) => {
    await toTrial(page);
    const shown = () =>
      page.evaluate(
        () => document.querySelector<HTMLImageElement>('.viewport img.shown')!.dataset.layer,
      );

    const rest = await shown();
    await page.keyboard.down('ArrowLeft');
    expect(await shown(), 'ArrowLeft holds A').toBe('a');
    await page.keyboard.up('ArrowLeft');
    expect(await shown(), 'releasing returns to rest').toBe(rest);

    await page.keyboard.down(' ');
    expect(await shown(), 'space peeks at the original').toBe('ref');
    await expect(page.locator('.trial.revealing')).toHaveCount(1);
    await page.keyboard.up(' ');
    expect(await shown(), 'releasing space returns to rest').toBe(rest);

    // A tap must not leave the view advanced.
    await page.keyboard.press('ArrowLeft');
    expect(await shown(), 'a tap must not latch').toBe(rest);
  });

  test('number keys rate a single-stimulus trial', async ({ page }) => {
    await toTrial(page);
    expect(await toKind(page, 'single'), 'needed a rating trial').toBe(true);
    const before = await page.locator('.trial').getAttribute('data-trial-id');
    await page.keyboard.press('2');
    // Advancing to a different trial is the observable effect of a recorded
    // response.
    await expect
      .poll(async () => page.locator('.trial').getAttribute('data-trial-id'), { timeout: 10_000 })
      .not.toBe(before);
  });

  test('a/b/c answer a pair trial and digits magnify it', async ({ page }) => {
    await toTrial(page);
    expect(await toKind(page, 'pair'), 'needed a pair trial').toBe(true);

    // Digits are free on pair trials (no 1-4 rating), so they drive the ladder.
    await page.keyboard.press('4');
    await expect(page.locator('#zoom-readout')).toHaveText('4×');
    // 3 and 5 exist now; the ladder is every whole factor, not powers of two.
    await page.keyboard.press('3');
    await expect(page.locator('#zoom-readout')).toHaveText('3×');
    await page.keyboard.press('0');
    await expect(page.locator('#zoom-readout')).toHaveText('1×');

    const before = await page.locator('.trial').getAttribute('data-trial-id');
    await page.keyboard.press('c'); // "can't tell"
    await expect
      .poll(async () => page.locator('.trial').getAttribute('data-trial-id'), { timeout: 10_000 })
      .not.toBe(before);
  });

  test('the keyboard help opens and closes', async ({ page }) => {
    await toTrial(page);
    await page.keyboard.press('?');
    await expect(page.locator('.key-help')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.locator('.key-help')).toHaveCount(0);
  });
});

test.describe('hold-to-compare mode', () => {
  // Splitting by half rather than by mouse button is what makes this work with
  // a thumb, so it must be offered on every device — including the phone
  // projects, which are the ones the study actually runs on.
  test('the mode is offered on every device', async ({ page }) => {
    await toTrial(page);
    await expect(page.locator('#input-mode')).toHaveCount(1);
    await expect(page.locator('#input-mode option[value="hold"]')).toHaveCount(1);
  });

  test('left half shows A, right half shows B, release shows the original', async ({ page }) => {
    await toTrial(page);
    expect(await toKind(page, 'pair'), 'needed a pair trial').toBe(true);

    await page.locator('#input-mode').selectOption('hold');
    await page.waitForSelector('.trial[data-input-mode="hold"]');
    await page.waitForSelector('.viewport:not(.is-loading)');

    const shown = () =>
      page.evaluate(
        () => document.querySelector<HTMLImageElement>('.viewport img.shown')!.dataset.layer,
      );
    expect(await shown(), 'the original is the resting view in hold mode').toBe('ref');

    const box = (await page.locator('#viewport').boundingBox())!;
    const y = box.y + box.height / 2;
    const leftX = box.x + box.width * 0.25;
    const rightX = box.x + box.width * 0.75;

    await page.mouse.move(leftX, y);
    await page.mouse.down();
    expect(await shown(), 'pressing the left half shows A').toBe('a');
    // The view switch tracks the hold — with no overlay on the picture, that
    // highlight is the observer's feedback about which variant they are seeing.
    await expect(page.locator('.view-switch button[data-view="a"]')).toHaveClass(/\bon\b/);
    await page.mouse.up();
    expect(await shown(), 'releasing returns to the original').toBe('ref');

    await page.mouse.move(rightX, y);
    await page.mouse.down();
    expect(await shown(), 'pressing the right half shows B').toBe('b');
    await expect(page.locator('.view-switch button[data-view="b"]')).toHaveClass(/\bon\b/);
    await page.mouse.up();
    expect(await shown()).toBe('ref');
  });

  // Panning has to keep working under a hold, and crossing the midline mid-drag
  // must NOT swap the variant — that would change the picture out from under a
  // comparison the observer is in the middle of making.
  test('the half is decided on press and survives a drag across the midline', async ({ page }) => {
    await toTrial(page);
    expect(await toKind(page, 'pair'), 'needed a pair trial').toBe(true);
    await page.locator('#input-mode').selectOption('hold');
    await page.waitForSelector('.trial[data-input-mode="hold"]');
    await page.waitForSelector('.viewport:not(.is-loading)');

    const shown = () =>
      page.evaluate(
        () => document.querySelector<HTMLImageElement>('.viewport img.shown')!.dataset.layer,
      );
    const box = (await page.locator('#viewport').boundingBox())!;
    const y = box.y + box.height / 2;

    await page.mouse.move(box.x + box.width * 0.2, y);
    await page.mouse.down();
    expect(await shown()).toBe('a');
    // Drag well past the centre into the right half.
    for (let i = 1; i <= 6; i++) {
      await page.mouse.move(box.x + box.width * (0.2 + i * 0.1), y);
    }
    expect(await shown(), 'crossing the midline must not swap A for B mid-gesture').toBe('a');
    await page.mouse.up();
    expect(await shown()).toBe('ref');
  });

  // A single-stimulus trial has no B, so the halves collapse to one gesture:
  // hold to see the encoding, release for the original.
  test('on a single-stimulus trial either half shows the compressed image', async ({ page }) => {
    await toTrial(page);
    expect(await toKind(page, 'single'), 'needed a rating trial').toBe(true);
    await page.locator('#input-mode').selectOption('hold');
    await page.waitForSelector('.trial[data-input-mode="hold"]');
    await page.waitForSelector('.viewport:not(.is-loading)');

    const shown = () =>
      page.evaluate(
        () => document.querySelector<HTMLImageElement>('.viewport img.shown')!.dataset.layer,
      );
    expect(await shown()).toBe('ref');

    const box = (await page.locator('#viewport').boundingBox())!;
    const y = box.y + box.height / 2;
    for (const frac of [0.25, 0.75]) {
      await page.mouse.move(box.x + box.width * frac, y);
      await page.mouse.down();
      expect(await shown(), `half at ${frac} shows the compressed image`).toBe('a');
      await page.mouse.up();
      expect(await shown()).toBe('ref');
    }
  });
});

test.describe('mouse-button mode', () => {
  test('is offered only where a mouse exists', async ({ page }, testInfo) => {
    await toTrial(page);
    const desktop = testInfo.project.name === 'chromium-desktop';
    await expect(page.locator('#input-mode option[value="buttons"]')).toHaveCount(
      desktop ? 1 : 0,
    );
    // `hold` covers the same idea with a thumb, so it is offered everywhere.
    await expect(page.locator('#input-mode option[value="hold"]')).toHaveCount(1);
  });

  test('left button shows A, right shows B, release shows the original', async ({
    page,
  }, testInfo) => {
    test.skip(testInfo.project.name !== 'chromium-desktop', 'needs a mouse');
    await toTrial(page);
    expect(await toKind(page, 'pair'), 'needed a pair trial').toBe(true);
    await page.locator('#input-mode').selectOption('buttons');
    await page.waitForSelector('.trial[data-input-mode="buttons"]');
    await page.waitForSelector('.viewport:not(.is-loading)');

    const shown = () =>
      page.evaluate(
        () => document.querySelector<HTMLImageElement>('.viewport img.shown')!.dataset.layer,
      );
    expect(await shown(), 'the original is the resting view').toBe('ref');

    const box = (await page.locator('#viewport').boundingBox())!;
    // Deliberately press on the RIGHT half with the LEFT button: in this mode
    // the button decides, not the position, and the two must not be confused.
    await page.mouse.move(box.x + box.width * 0.8, box.y + box.height / 2);
    await page.mouse.down({ button: 'left' });
    expect(await shown(), 'left button shows A wherever the pointer is').toBe('a');
    await page.mouse.up({ button: 'left' });
    expect(await shown()).toBe('ref');

    await page.mouse.move(box.x + box.width * 0.2, box.y + box.height / 2);
    await page.mouse.down({ button: 'right' });
    expect(await shown(), 'right button shows B wherever the pointer is').toBe('b');
    await page.mouse.up({ button: 'right' });
    expect(await shown()).toBe('ref');
  });
});

test.describe('wheel magnification', () => {
  test('the wheel steps through whole factors and never lands between them', async ({
    page,
  }, testInfo) => {
    // A phone has no scroll wheel, and the mobile-emulated projects do not
    // deliver wheel events at all. Scoped by project, which is a property of the
    // device being emulated, not a runtime bail-out.
    test.skip(testInfo.project.name !== 'chromium-desktop', 'needs a scroll wheel');
    await toTrial(page);
    const box = (await page.locator('#viewport').boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);

    const factor = async () =>
      page.evaluate(() => {
        const i = document.querySelector<HTMLImageElement>('#stimulus')!;
        return (i.getBoundingClientRect().width * window.devicePixelRatio) / i.naturalWidth;
      });

    // Do not assume 1x: a stimulus smaller than the frame is magnified to
    // cover it on load, so the starting factor depends on the source drawn.
    const start = Number((await page.locator('#zoom-readout').textContent())!.replace('×', ''));
    expect(start).toBeGreaterThanOrEqual(1);
    for (const want of [start + 1, start + 2, start + 3].filter((z) => z <= 8)) {
      await page.mouse.wheel(0, -120); // one notch in
      await page.waitForTimeout(120);
      await expect(page.locator('#zoom-readout')).toHaveText(`${want}×`);
      const f = await factor();
      // The whole point of snapping: a fractional factor would resample the
      // stimulus, which is what the 1:1 rule exists to prevent.
      expect(Math.abs(f - Math.round(f)), `factor ${f} must be a whole number`).toBeLessThan(
        0.02,
      );
    }
    const top = Number((await page.locator('#zoom-readout').textContent())!.replace('×', ''));
    for (const want of [top - 1, top - 2].filter((z) => z >= 1)) {
      await page.mouse.wheel(0, 120); // one notch out
      await page.waitForTimeout(120);
      await expect(page.locator('#zoom-readout')).toHaveText(`${want}×`);
    }
    // Never below 1:1, however hard you scroll out.
    for (let i = 0; i < 5; i++) await page.mouse.wheel(0, 120);
    await page.waitForTimeout(120);
    await expect(page.locator('#zoom-readout')).toHaveText('1×');
    expect(await factor()).toBeGreaterThanOrEqual(0.98);
  });
});

test.describe('surround indicator', () => {
  // The letterbox around the stimulus carried no information, while the only
  // persistent cue for which variant was on screen was a button below the
  // frame. Under hold/buttons the variant changes as fast as you can press, so
  // the answer has to be visible without looking away from the picture.
  test('the surround is tiled with the variant currently shown', async ({ page }) => {
    await toTrial(page);
    expect(await toKind(page, 'pair'), 'needed a pair trial').toBe(true);

    const surround = () =>
      page.evaluate(() => {
        const vp = document.querySelector<HTMLElement>('#viewport')!;
        return {
          view: vp.dataset.view,
          image: getComputedStyle(vp).backgroundImage,
        };
      });

    const seen: Record<string, string> = {};
    for (const v of ['a', 'b', 'ref'] as const) {
      await page.locator(`.view-switch button[data-view="${v}"]`).click();
      const s = await surround();
      expect(s.view, 'the viewport must advertise the current variant').toBe(v);
      expect(s.image, `no surround pattern for ${v}`).not.toBe('none');
      seen[v] = s.image;
    }
    // Three distinct patterns — a shared one would label nothing.
    expect(new Set(Object.values(seen)).size).toBe(3);
  });

  // The surround is behind a psychovisual stimulus. A bright or tinted pattern
  // would change local adaptation and bias colour judgements, so the glyph
  // carries the meaning and the ink stays dark and neutral.
  test('the pattern is dark and neutral, never coloured', async ({ page }) => {
    await toTrial(page);
    const ink = await page.evaluate(() => {
      const vp = document.querySelector<HTMLElement>('#viewport')!;
      const img = getComputedStyle(vp).backgroundImage;
      const m = decodeURIComponent(img).match(/fill="#([0-9a-fA-F]{6})"/);
      return m ? m[1] : null;
    });
    expect(ink, 'expected an inline SVG tile with an explicit fill').not.toBeNull();
    const r = parseInt(ink!.slice(0, 2), 16);
    const g = parseInt(ink!.slice(2, 4), 16);
    const b = parseInt(ink!.slice(4, 6), 16);
    expect(Math.max(r, g, b) - Math.min(r, g, b), 'must be neutral grey').toBe(0);
    expect(r, 'must stay dark so it cannot shift adaptation').toBeLessThan(0x60);
  });
});

test.describe('no flash on switch', () => {
  // "Always preload both A and B prior to click." The panel used to unlock as
  // soon as the *judged* layer arrived, so on a pair trial you could press B
  // while B was still on the wire — a real source is ~9.5 MB, so that is a
  // blank viewport, not a flicker.
  test('the panel stays locked until every variant has arrived', async ({ page }) => {
    await gotoFresh(page);
    await clickBegin(page);
    await page.getByRole('button', { name: /^Skip$/ }).click();

    // Hold the reference (and only the reference) on the wire.
    let release: () => void = () => {};
    const gate = new Promise<void>((r) => (release = r));
    let gated = 0;
    await page.route('**/api/proxy/source/**', async (route) => {
      gated += 1;
      await gate;
      await route.continue();
    });

    await completeProfileAndStart(page);
    await page.waitForSelector('.trial[data-trial-id]');
    // Give the encodings time to land while the reference is still blocked.
    await page.waitForTimeout(600);

    const mid = await page.evaluate(() => {
      const btn = document.querySelector<HTMLButtonElement>(
        '.rating-panel button, .pair-panel button',
      );
      const vp = document.querySelector<HTMLElement>('#viewport')!;
      return {
        disabled: btn?.disabled ?? null,
        allReady: vp.classList.contains('all-ready'),
        decoded: [...document.querySelectorAll<HTMLImageElement>('.viewport img.layer')].map(
          (i) => i.naturalWidth > 0,
        ),
      };
    });
    expect(gated, 'the reference request should have been intercepted').toBeGreaterThan(0);
    expect(mid.allReady, 'not all variants are in yet').toBe(false);
    expect(mid.disabled, 'answering must be impossible until every variant is in').toBe(true);
    expect(mid.decoded.some((d) => !d), 'a layer should still be outstanding').toBe(true);

    release();
    await page.waitForSelector('.viewport.all-ready', { timeout: 20_000 });
    const done = await page.evaluate(() => ({
      disabled: document.querySelector<HTMLButtonElement>(
        '.rating-panel button, .pair-panel button',
      )!.disabled,
      decoded: [...document.querySelectorAll<HTMLImageElement>('.viewport img.layer')].every(
        (i) => i.naturalWidth > 0,
      ),
    }));
    expect(done.decoded, 'every variant decoded').toBe(true);
    expect(done.disabled, 'unlocked once everything is in').toBe(false);
  });

  // Hiding by `visibility` skips painting entirely, so the first flip has to
  // rasterise on the spot — one dropped frame, seen as a flash between the two
  // pictures being compared. Opacity + compositor promotion makes the swap a
  // compositor property change instead.
  test('hidden variants stay composited so a swap cannot repaint', async ({ page }) => {
    await toTrial(page);
    await page.waitForSelector('.viewport.all-ready');
    const styles = await page.evaluate(() =>
      [...document.querySelectorAll<HTMLImageElement>('.viewport img.layer')].map((i) => {
        const cs = getComputedStyle(i);
        return {
          shown: i.classList.contains('shown'),
          opacity: cs.opacity,
          visibility: cs.visibility,
          willChange: cs.willChange,
        };
      }),
    );
    for (const s of styles) {
      expect(s.visibility, 'layers must stay visible to remain painted').toBe('visible');
      expect(s.opacity, 'hidden layers are transparent, shown ones opaque').toBe(
        s.shown ? '1' : '0',
      );
      expect(s.willChange, 'each layer needs its own compositor layer').toContain('opacity');
    }
    // Exactly 0 — a faintly visible second variant would composite over the
    // stimulus under test, which is worse than any flash.
    expect(styles.filter((s) => !s.shown).every((s) => s.opacity === '0')).toBe(true);
  });
});

test.describe('default input mode', () => {
  // On a phone the segmented control is three small targets below the picture,
  // and every switch is a look away from the thing being compared. Holding one
  // half keeps the eye on the stimulus and changes the picture under it.
  test('touch devices start in hold mode, mouse devices in tap', async ({ page }, testInfo) => {
    await toTrial(page);
    const mode = await page.evaluate(
      () => document.querySelector<HTMLElement>('.trial')!.dataset.inputMode,
    );
    const coarse = testInfo.project.name !== 'chromium-desktop';
    expect(mode, coarse ? 'touch should default to hold' : 'mouse should default to tap').toBe(
      coarse ? 'hold' : 'tap',
    );
  });

  // An explicit choice is what makes the setting stick — it must outrank the
  // device default on every later visit.
  test('an explicit choice outlives a reload', async ({ page }, testInfo) => {
    await toTrial(page);
    const other = testInfo.project.name === 'chromium-desktop' ? 'hold' : 'tap';
    await page.locator('#input-mode').selectOption(other);
    await page.waitForSelector(`.trial[data-input-mode="${other}"]`);

    await page.reload();
    await page.waitForSelector('.trial[data-trial-id]', { timeout: 30_000 });
    const after = await page.evaluate(
      () => document.querySelector<HTMLElement>('.trial')!.dataset.inputMode,
    );
    expect(after, 'the chosen mode must survive a reload').toBe(other);
  });
});

test.describe('hold ordering through the UI', () => {
  // The stack's unit tests prove the logic; this proves it is actually wired to
  // the mouse, in every mode, including the fall-back-on-release cases.
  for (const mode of ['tap', 'hold', 'buttons'] as const) {
    test(`right button shows B in ${mode} mode, and releasing falls back`, async ({
      page,
    }, testInfo) => {
      test.skip(testInfo.project.name !== 'chromium-desktop', 'needs mouse buttons');
      await toTrial(page);
      expect(await toKind(page, 'pair'), 'needed a pair trial').toBe(true);
      await page.locator('#input-mode').selectOption(mode);
      await page.waitForSelector(`.trial[data-input-mode="${mode}"]`);
      await page.waitForSelector('.viewport.all-ready');

      const shown = () =>
        page.evaluate(
          () => document.querySelector<HTMLImageElement>('.viewport img.shown')!.dataset.layer,
        );
      const box = (await page.locator('#viewport').boundingBox())!;
      // Deliberately the LEFT half, so `hold` mode's positional left button
      // does not coincide with what the right button does.
      const x = box.x + box.width * 0.25;
      const y = box.y + box.height / 2;
      await page.mouse.move(x, y);

      const rest = await shown();

      // Right button → B, regardless of mode or position.
      await page.mouse.down({ button: 'right' });
      expect(await shown(), `right button in ${mode}`).toBe('b');
      await page.mouse.up({ button: 'right' });
      expect(await shown(), 'released → resting').toBe(rest);

      // LMB held, then RMB → B; release RMB → back to whatever LMB shows.
      await page.mouse.down({ button: 'left' });
      const underLeft = await shown();
      await page.mouse.down({ button: 'right' });
      expect(await shown(), 'newest hold wins').toBe('b');
      await page.mouse.up({ button: 'right' });
      expect(await shown(), 'LMB still down → falls back to it').toBe(underLeft);
      await page.mouse.up({ button: 'left' });
      expect(await shown(), 'all released → resting').toBe(rest);
    });
  }

  test('RMB held, LMB pressed and released, returns to B', async ({ page }, testInfo) => {
    test.skip(testInfo.project.name !== 'chromium-desktop', 'needs mouse buttons');
    await toTrial(page);
    expect(await toKind(page, 'pair'), 'needed a pair trial').toBe(true);
    await page.locator('#input-mode').selectOption('buttons');
    await page.waitForSelector('.trial[data-input-mode="buttons"]');
    await page.waitForSelector('.viewport.all-ready');

    const shown = () =>
      page.evaluate(
        () => document.querySelector<HTMLImageElement>('.viewport img.shown')!.dataset.layer,
      );
    const box = (await page.locator('#viewport').boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);

    await page.mouse.down({ button: 'right' });
    expect(await shown()).toBe('b');
    await page.mouse.down({ button: 'left' });
    expect(await shown()).toBe('a');
    await page.mouse.up({ button: 'left' });
    expect(await shown(), 'RMB still down → back to B').toBe('b');

    // Releasing the *underneath* hold while A is on top changes nothing...
    await page.mouse.down({ button: 'left' });
    expect(await shown()).toBe('a');
    await page.mouse.up({ button: 'right' });
    expect(await shown(), 'A was on top').toBe('a');
    // ...and pressing it again puts B back on top.
    await page.mouse.down({ button: 'right' });
    expect(await shown()).toBe('b');
    await page.mouse.up({ button: 'right' });
    await page.mouse.up({ button: 'left' });
  });

  // Arrows are HELD now, not tapped — they used to step a carousel.
  test('arrow keys hold rather than toggle, and stack with each other', async ({ page }) => {
    await toTrial(page);
    expect(await toKind(page, 'pair'), 'needed a pair trial').toBe(true);
    const shown = () =>
      page.evaluate(
        () => document.querySelector<HTMLImageElement>('.viewport img.shown')!.dataset.layer,
      );
    const rest = await shown();

    await page.keyboard.down('ArrowRight');
    expect(await shown(), 'ArrowRight holds B').toBe('b');
    await page.keyboard.down('ArrowLeft');
    expect(await shown(), 'newest hold wins').toBe('a');
    await page.keyboard.up('ArrowLeft');
    expect(await shown(), 'ArrowRight still down').toBe('b');
    await page.keyboard.up('ArrowRight');
    expect(await shown(), 'released → resting').toBe(rest);

    // Held, not toggled: pressing and releasing returns to rest rather than
    // leaving the view advanced.
    await page.keyboard.press('ArrowRight');
    expect(await shown(), 'a tap must not latch').toBe(rest);
  });
});
