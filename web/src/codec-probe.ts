// Codec support detection. Phone observers ride a wide range of native decoders:
// JXL is behind a flag in Chrome (chrome://flags/#enable-jxl-image-format) and
// missing on Firefox; AVIF is mostly there but spotty on older Safari; WebP is
// universal. JPEG and PNG we assume.
//
// We probe with 1×1 data-URL blobs, then send the result back to the server so
// the trial sampler never serves a codec the observer can't natively render —
// transcoding server-side would mean we're measuring the wrong pipeline.

const PROBES: Array<{ codec: string; mime: string; b64: string }> = [
  // 1×1 JXL (squoosh's canonical probe blob).
  { codec: 'jxl', mime: 'image/jxl', b64: '/wr6HwGRCAYBAGAASzgkun4ANwA=' },
  // 1×1 AVIF (av1-encoded, color box minimal).
  {
    codec: 'avif',
    mime: 'image/avif',
    b64:
      'AAAAHGZ0eXBhdmlmAAAAAGF2aWZtaWYxbWlhZgAAAOptZXRhAAAAAAAAACFoZGxyAAA' +
      'AAAAAAABwaWN0AAAAAAAAAAAAAAAAAAAAAA5waXRtAAAAAAABAAAAImlsb2MAAAAARA' +
      'AAAAEAAQAAAAEAAAEKAAAAGwAAACNpaW5mAAAAAAABAAAAFWluZmUCAAAAAAEAAGF2M' +
      'DEAAAAAamlwcnAAAABLaXBjbwAAABRpc3BlAAAAAAAAAAEAAAABAAAAEHBpeGkAAAAA' +
      'AwgICAAAAAxhdjFDgQAMAAAAABNjb2xybmNseAACAAIABoAAAAAXaXBtYQAAAAAAAAA' +
      'BAAEEgQIDhAAAABptZGF0EgAKBzgAACjFCSAESDIgaqAAcGqs',
  },
  // 1×1 WebP (lossless, well-known reference blob).
  {
    codec: 'webp',
    mime: 'image/webp',
    b64: 'UklGRhoAAABXRUJQVlA4TA0AAAAvAAAAEAcQERGIiP4HAA==',
  },
];

// JPEG and PNG are universal; we never gate on them.
const ALWAYS_SUPPORTED = ['jpeg', 'mozjpeg', 'png'];

const STORAGE_KEY = 'squintly:codec_support_v1';
const STORAGE_TTL_MS = 7 * 24 * 60 * 60 * 1000; // 7 days; Chrome flag toggles change it

interface CachedProbe {
  ts: number;
  ua: string;
  supported: string[];
}

async function probeOne(mime: string, b64: string): Promise<boolean> {
  return new Promise((resolve) => {
    const img = new Image();
    let settled = false;
    const finish = (ok: boolean) => {
      if (settled) return;
      settled = true;
      resolve(ok);
    };
    img.onload = () => finish(img.naturalWidth > 0 && img.naturalHeight > 0);
    img.onerror = () => finish(false);
    // Image.decode() is the cleanest path on modern browsers, but onload/onerror
    // is the most portable. Race a 1500 ms timeout so a hang on one codec doesn't
    // wedge the whole probe.
    setTimeout(() => finish(false), 1500);
    img.src = `data:${mime};base64,${b64}`;
  });
}

export interface CodecSupport {
  supported: Set<string>;
  // True if we trust the cached value; false if we just freshly probed.
  cached: boolean;
}

export async function detectCodecs(force = false): Promise<CodecSupport> {
  if (!force) {
    try {
      const raw = localStorage.getItem(STORAGE_KEY);
      if (raw) {
        const parsed = JSON.parse(raw) as CachedProbe;
        if (
          parsed &&
          parsed.ua === navigator.userAgent &&
          Date.now() - parsed.ts < STORAGE_TTL_MS &&
          Array.isArray(parsed.supported)
        ) {
          return { supported: new Set([...ALWAYS_SUPPORTED, ...parsed.supported]), cached: true };
        }
      }
    } catch {
      // ignore parse errors; fall through and re-probe
    }
  }

  const results = await Promise.all(
    PROBES.map(async (p) => ({ codec: p.codec, ok: await probeOne(p.mime, p.b64) })),
  );
  const supported = results.filter((r) => r.ok).map((r) => r.codec);
  try {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ ts: Date.now(), ua: navigator.userAgent, supported } satisfies CachedProbe),
    );
  } catch {
    // storage may be full or denied; that's fine
  }
  return { supported: new Set([...ALWAYS_SUPPORTED, ...supported]), cached: false };
}

export function describeMissing(supported: Set<string>): string[] {
  const all = ['jxl', 'avif', 'webp'];
  return all.filter((c) => !supported.has(c));
}

/// Actionable form of the hint, for the front page.
///
/// `jxlEnableHint` returns a sentence sized for a one-line onboarding notice.
/// The front page can afford to be instructional instead: somebody deciding
/// whether to take part is exactly who can spare thirty seconds to flip a flag,
/// whereas somebody mid-session cannot. `flag` is a copyable URL when there is
/// one, so the instruction is followable rather than merely informative.
export interface JxlAdvice {
  /// True when the browser already decodes JXL — render nothing.
  supported: boolean;
  headline: string;
  detail: string;
  /// e.g. `chrome://flags/#enable-jxl-image-format`. Null when no flag exists.
  flag: string | null;
}

export function jxlAdvice(supported: Set<string>): JxlAdvice {
  if (supported.has('jxl')) {
    return { supported: true, headline: '', detail: '', flag: null };
  }
  const ua = navigator.userAgent;
  if (/Firefox\//.test(ua)) {
    return {
      supported: false,
      headline: 'Firefox cannot show JPEG XL yet',
      detail:
        'You can still take part — those comparisons are simply skipped for you. ' +
        'Chrome with a flag enabled, or Safari on iOS 17+, covers them.',
      flag: null,
    };
  }
  if (/Chrome\/|Chromium\//.test(ua)) {
    // Edge and Brave are Chromium and take the same flag, so they are not
    // special-cased out of it — the earlier version excluded Edg/ and left
    // those users with the generic "your browser cannot" message despite the
    // flag working for them.
    const flag = /Edg\//.test(ua)
      ? 'edge://flags/#enable-jxl-image-format'
      : 'chrome://flags/#enable-jxl-image-format';
    return {
      supported: false,
      headline: 'Turn on JPEG XL to rate the newest codec',
      detail:
        'Paste the address below into a new tab, set it to Enabled, then restart the ' +
        'browser and come back. Without it those comparisons are skipped for you.',
      flag,
    };
  }
  if (/Safari\//.test(ua)) {
    return {
      supported: false,
      headline: 'This Safari cannot show JPEG XL',
      detail: 'Safari 17 and later can. Until then those comparisons are skipped for you.',
      flag: null,
    };
  }
  return {
    supported: false,
    headline: 'This browser cannot show JPEG XL',
    detail: 'Those comparisons will be skipped for you; everything else works normally.',
    flag: null,
  };
}

export function jxlEnableHint(supported: Set<string>): string | null {
  if (supported.has('jxl')) return null;
  // Chrome / Edge / Brave behind a flag; Firefox no support yet.
  const ua = navigator.userAgent;
  if (/Firefox\//.test(ua)) {
    return 'Firefox does not yet support JPEG XL; we will skip JXL trials for you.';
  }
  if (/Chrome\/|Chromium\//.test(ua) && !/Edg\//.test(ua)) {
    return 'Want to rate JPEG XL too? Open chrome://flags/#enable-jxl-image-format, enable it, restart, and reload this page.';
  }
  if (/Safari\//.test(ua) && !/Chrome\//.test(ua)) {
    return 'Your Safari does not appear to support JPEG XL — try iOS 17+ to include JXL trials.';
  }
  return 'This browser does not appear to support JPEG XL; we will skip JXL trials for you.';
}
