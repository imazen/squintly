// Tiny HTTP server matching coefficient's contract (see ../../docs or
// `src/coefficient.rs` for the schemas). Serves a stable manifest + bytes
// for a handful of (source, codec, quality) cells using base64-encoded 1×1
// blobs that real browsers can decode.
//
// Port and shape come from playwright.config.ts. Launched by global-setup.ts.

import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import { deflateSync } from 'node:zlib';

const PORT = Number(process.env.COEFFICIENT_PORT ?? 18081);

// ---------------------------------------------------------------------------
// Tiny PNG writer — real, decodable, structured test images so trials and the
// curator threshold screens show actual pixels instead of a 1×1 dot. Truecolor
// 8-bit, filter 0 rows, one IDAT. Deterministic per (seed, w, h, levels).
// `levels` posterizes the channels: q90 ≈ smooth, q10 ≈ heavy banding — so
// different qualities are visibly different in pair trials.
// ---------------------------------------------------------------------------

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

function crc32(buf: Buffer): number {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) c = CRC_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function pngChunk(type: string, data: Buffer): Buffer {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

function encodePng(w: number, h: number, rgb: (x: number, y: number) => [number, number, number]): Buffer {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 2; // truecolor
  const raw = Buffer.alloc(h * (1 + w * 3));
  let o = 0;
  for (let y = 0; y < h; y++) {
    raw[o++] = 0; // filter 0
    for (let x = 0; x < w; x++) {
      const [r, g, b] = rgb(x, y);
      raw[o++] = r;
      raw[o++] = g;
      raw[o++] = b;
    }
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    pngChunk('IHDR', ihdr),
    pngChunk('IDAT', deflateSync(raw, { level: 6 })),
    pngChunk('IEND', Buffer.alloc(0)),
  ]);
}

/** Structured scene: gradient sky + rings + high-frequency checker strip —
 * enough spatial variety that posterization and JPEG artifacts are visible. */
function testImage(seed: number, w: number, h: number, levels: number): Buffer {
  const step = 255 / Math.max(1, levels - 1);
  const post = (v: number) => Math.round(Math.max(0, Math.min(255, v)) / step) * step;
  const cx = w * (0.35 + 0.3 * ((seed * 37) % 100) / 100);
  const cy = h * (0.4 + 0.2 * ((seed * 53) % 100) / 100);
  return encodePng(w, h, (x, y) => {
    const d = Math.hypot(x - cx, y - cy);
    const ring = 127 + 127 * Math.sin(d / (8 + (seed % 5)));
    const grad = (255 * y) / h;
    const checker = y > h * 0.8 ? (((x >> 2) + (y >> 2)) & 1 ? 235 : 20) : null;
    const r = checker ?? (0.55 * ring + 0.45 * grad);
    const g = checker ?? (0.35 * ring + 0.65 * (255 - grad));
    const b = checker ?? (0.7 * grad + 0.3 * (255 - ring));
    return [post(r), post(g), post(b)];
  });
}

const imageCache = new Map<string, Buffer>();
function cachedImage(key: string, seed: number, w: number, h: number, levels: number): Buffer {
  let buf = imageCache.get(key);
  if (!buf) {
    buf = testImage(seed, w, h, levels);
    imageCache.set(key, buf);
  }
  return buf;
}

/** Map an encoding quality (10..90) to posterize levels (4..64). */
function levelsForQuality(q: number): number {
  return Math.max(4, Math.min(64, Math.round((q / 90) * 64)));
}

// Codec NAMES matter (the JXL codec-probe filter test relies on zenjxl
// manifest entries existing while Chromium can't decode JXL); the served
// BYTES are always generated PNGs — browsers render by sniffing, and the
// sampler filters by manifest codec_name, never by response mime.

interface SourceMeta { hash: string; width: number; height: number; size_bytes: number; corpus: string; filename: string }
interface EncodingMeta { id: string; source_hash: string; codec_name: string; quality: number; encoded_size: number }

function buildManifest() {
  const sources: SourceMeta[] = [
    { hash: 'src01', width: 256, height: 256, size_bytes: 24_000, corpus: 'test', filename: 'a.png' },
    { hash: 'src02', width: 1024, height: 1024, size_bytes: 310_000, corpus: 'test', filename: 'b.png' },
    { hash: 'src03', width: 512, height: 384, size_bytes: 61_000, corpus: 'test', filename: 'c.png' },
  ];
  const codecs: Array<{ name: string; mime: string; ext: string }> = [
    { name: 'mozjpeg', mime: 'image/jpeg', ext: 'jpeg' },
    { name: 'zenwebp', mime: 'image/webp', ext: 'webp' },
    { name: 'zenavif', mime: 'image/avif', ext: 'avif' },
    { name: 'zenjxl',  mime: 'image/jxl',  ext: 'jxl'  },
  ];
  const qualities = [10, 30, 50, 70, 90];
  const encodings: EncodingMeta[] = [];
  for (const src of sources) {
    for (const codec of codecs) {
      for (const q of qualities) {
        encodings.push({
          id: `${src.hash}__${codec.name}__q${q}`,
          source_hash: src.hash,
          codec_name: codec.name,
          quality: q,
          encoded_size: 100 + q * 10,
        });
      }
    }
  }
  return { sources, encodings };
}

const manifest = buildManifest();

function send(res: ServerResponse, status: number, body: Buffer | string, contentType: string) {
  res.statusCode = status;
  res.setHeader('content-type', contentType);
  res.setHeader('cache-control', 'public, max-age=300');
  // DELIBERATELY no access-control-allow-origin. The canonical R2 bucket
  // (pub-….r2.dev) sends none (measured 2026-07-27), so neither do we — a mock
  // that is more permissive than production hides exactly the bug we hit: any
  // canvas-reading path pointed straight at a candidate's blob_url fails to
  // load. Those paths must go through the same-origin /api/curator/blob/{sha}
  // proxy, and keeping the mock CORS-less is what keeps e2e honest about it.
  res.end(body);
}

// Curator-mode test fixtures, content-addressed under R2-shaped paths so a
// JSONL manifest pointed at this mock with `blob_url_base =
// http://127.0.0.1:<port>` resolves to real bytes. The fixture sha256 values
// are the canonical R2 layout (`/blobs/xx/yy/{sha}`) and match
// `web/e2e/curator-fixture.jsonl`. Each sha maps to a deterministic generated
// scene big enough for the 1:1 threshold panels to show real pixels.
const CURATOR_FIXTURE_DIMS: Record<string, { seed: number; w: number; h: number }> = {
  'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa': { seed: 1, w: 1200, h: 880 },
  'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb': { seed: 2, w: 900, h: 1200 },
  'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc': { seed: 3, w: 800, h: 600 },
};

function blobByPathSegments(p: string): { buf: Buffer; mime: string } | null {
  // Match either /blobs/xx/yy/{sha} (R2 layout) or /blobs/{sha} flat.
  const m = p.match(/^\/blobs\/(?:[0-9a-f]{2}\/[0-9a-f]{2}\/)?([0-9a-f]{32,})$/i);
  if (!m) return null;
  const sha = m[1].toLowerCase();
  const spec = CURATOR_FIXTURE_DIMS[sha] ?? { seed: 9, w: 640, h: 480 };
  return {
    buf: cachedImage(`blob:${sha}`, spec.seed, spec.w, spec.h, 64),
    mime: 'image/png',
  };
}

const server = createServer((req: IncomingMessage, res: ServerResponse) => {
  const url = new URL(req.url ?? '/', `http://127.0.0.1:${PORT}`);
  if (url.pathname === '/api/manifest') {
    send(res, 200, JSON.stringify(manifest), 'application/json');
    return;
  }
  let m = url.pathname.match(/^\/api\/sources\/([^/]+)\/image$/);
  if (m) {
    const src = manifest.sources.find((s) => s.hash === m![1]);
    const seed = src ? manifest.sources.indexOf(src) + 1 : 9;
    send(
      res,
      200,
      cachedImage(`src:${m[1]}`, seed, src?.width ?? 640, src?.height ?? 480, 64),
      'image/png',
    );
    return;
  }
  m = url.pathname.match(/^\/api\/encodings\/([^/]+)\/image$/);
  if (m) {
    // Serve a posterized variant of the source scene: lower q → fewer levels
    // → visible banding. Bytes are PNG regardless of the declared codec (the
    // sampler filters by manifest codec_name, not by response mime), so pair
    // trials show a real, quality-dependent difference.
    const id = m[1];
    const [srcHash, , qPart] = id.split('__');
    const src = manifest.sources.find((s) => s.hash === srcHash);
    const seed = src ? manifest.sources.indexOf(src) + 1 : 9;
    const q = Number((qPart ?? 'q50').replace(/^q/, '')) || 50;
    send(
      res,
      200,
      cachedImage(`enc:${srcHash}:q${q}`, seed, src?.width ?? 640, src?.height ?? 480, levelsForQuality(q)),
      'image/png',
    );
    return;
  }
  // Content-addressed blob URLs for the curator manifest.
  const blob = blobByPathSegments(url.pathname);
  if (blob) {
    send(res, 200, blob.buf, blob.mime);
    return;
  }
  if (url.pathname === '/health') {
    send(res, 200, 'ok', 'text/plain');
    return;
  }
  send(res, 404, 'not found', 'text/plain');
});

server.listen(PORT, '127.0.0.1', () => {
  // eslint-disable-next-line no-console
  console.log(`[mock-coefficient] listening on http://127.0.0.1:${PORT}`);
});

// Be polite on signals so global-teardown's kill is clean.
for (const sig of ['SIGINT', 'SIGTERM']) {
  process.on(sig, () => {
    server.close(() => process.exit(0));
  });
}
