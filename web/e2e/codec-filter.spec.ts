import { expect, test } from '@playwright/test';

import {
  clickBegin,
  completeProfileAndStart,
  gotoFresh,
  submitOneTrial,
} from './helpers';

// Headless Chromium does not natively decode JXL. The probe should mark JXL
// unsupported, the session creates with supported_codecs lacking JXL, and the
// sampler must therefore never serve a zenjxl encoding to this observer.

test('codec probe filters JXL trials away from Chromium without the flag', async ({ page, request }) => {
  await gotoFresh(page);
  await clickBegin(page);
  await page.getByRole('button', { name: /^Skip$/ }).click();
  await completeProfileAndStart(page);

  // Rate ~20 trials. The codec of every served encoding is captured server-side
  // and surfaces in the responses TSV — we'll inspect that, not the DOM.
  const seenCodecs = new Set<string>();
  for (let i = 0; i < 20; i++) {
    await submitOneTrial(page);
  }

  // Now query the responses TSV — it carries the codec of every trial served.
  const tsv = await (await request.get('/api/export/responses.tsv')).text();
  const lines = tsv.split('\n').filter((l) => l.length);
  const header = lines.shift()!.split('\t');
  const aCodecIdx = header.indexOf('a_codec');
  const bCodecIdx = header.indexOf('b_codec');
  expect(aCodecIdx).toBeGreaterThanOrEqual(0);
  for (const line of lines) {
    const cols = line.split('\t');
    if (cols[aCodecIdx]) seenCodecs.add(cols[aCodecIdx]);
    if (cols[bCodecIdx]) seenCodecs.add(cols[bCodecIdx]);
  }
  // Hard guarantee: no JXL encodings should have been served to this Chromium.
  for (const codec of seenCodecs) {
    expect(codec.includes('jxl')).toBe(false);
  }
  // And we should have seen at least one codec at all.
  expect(seenCodecs.size).toBeGreaterThan(0);
});
