// Tiny HTTP server matching coefficient's contract (see ../../docs or
// `src/coefficient.rs` for the schemas). Serves a stable manifest + bytes
// for a handful of (source, codec, quality) cells using base64-encoded 1×1
// blobs that real browsers can decode.
//
// Port and shape come from playwright.config.ts. Launched by global-setup.ts.

import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';

const PORT = Number(process.env.COEFFICIENT_PORT ?? 18081);

// 1×1 blobs, all decodable by browsers. JXL is here too even though Chromium
// in CI doesn't decode it — that's the whole point of the codec-probe filter
// test (the binary still serves manifest entries; the frontend just shouldn't
// pick them).
const ONE_BY_ONE = {
  png: Buffer.from(
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=',
    'base64',
  ),
  // Tiny JPEG (smallest valid JFIF I have on hand).
  jpeg: Buffer.from(
    '/9j/4AAQSkZJRgABAQEAYABgAAD/2wBDAAEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQH/2wBDAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQH/wAARCAABAAEDASIAAhEBAxEB/8QAFQABAQAAAAAAAAAAAAAAAAAAAAr/xAAUEAEAAAAAAAAAAAAAAAAAAAAA/8QAFAEBAAAAAAAAAAAAAAAAAAAAAP/EABQRAQAAAAAAAAAAAAAAAAAAAAD/2gAMAwEAAhEDEQA/AL+ABn//2Q==',
    'base64',
  ),
  webp: Buffer.from('UklGRhoAAABXRUJQVlA4TA0AAAAvAAAAEAcQERGIiP4HAA==', 'base64'),
  avif: Buffer.from(
    'AAAAHGZ0eXBhdmlmAAAAAGF2aWZtaWYxbWlhZgAAAOptZXRhAAAAAAAAACFoZGxyAAAAAAAAAABwaWN0AAAAAAAAAAAAAAAAAAAAAA5waXRtAAAAAAABAAAAImlsb2MAAAAARAAAAAEAAQAAAAEAAAEKAAAAGwAAACNpaW5mAAAAAAABAAAAFWluZmUCAAAAAAEAAGF2MDEAAAAAamlwcnAAAABLaXBjbwAAABRpc3BlAAAAAAAAAAEAAAABAAAAEHBpeGkAAAAAAwgICAAAAAxhdjFDgQAMAAAAABNjb2xybmNseAACAAIABoAAAAAXaXBtYQAAAAAAAAABAAEEgQIDhAAAABptZGF0EgAKBzgAACjFCSAESDIgaqAAcGqs',
    'base64',
  ),
  jxl: Buffer.from('/wr6HwGRCAYBAGAASzgkun4ANwA=', 'base64'),
};

interface SourceMeta { hash: string; width: number; height: number; size_bytes: number; corpus: string; filename: string }
interface EncodingMeta { id: string; source_hash: string; codec_name: string; quality: number; encoded_size: number }

function buildManifest() {
  const sources: SourceMeta[] = [
    { hash: 'src01', width: 256, height: 256, size_bytes: ONE_BY_ONE.png.length, corpus: 'test', filename: 'a.png' },
    { hash: 'src02', width: 1024, height: 1024, size_bytes: ONE_BY_ONE.png.length, corpus: 'test', filename: 'b.png' },
    { hash: 'src03', width: 512, height: 384, size_bytes: ONE_BY_ONE.png.length, corpus: 'test', filename: 'c.png' },
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
  res.end(body);
}

function blobForCodec(codec: string): { buf: Buffer; mime: string } {
  if (codec.includes('jxl')) return { buf: ONE_BY_ONE.jxl, mime: 'image/jxl' };
  if (codec.includes('avif')) return { buf: ONE_BY_ONE.avif, mime: 'image/avif' };
  if (codec.includes('webp')) return { buf: ONE_BY_ONE.webp, mime: 'image/webp' };
  if (codec.includes('png')) return { buf: ONE_BY_ONE.png, mime: 'image/png' };
  return { buf: ONE_BY_ONE.jpeg, mime: 'image/jpeg' };
}

const server = createServer((req: IncomingMessage, res: ServerResponse) => {
  const url = new URL(req.url ?? '/', `http://127.0.0.1:${PORT}`);
  if (url.pathname === '/api/manifest') {
    send(res, 200, JSON.stringify(manifest), 'application/json');
    return;
  }
  let m = url.pathname.match(/^\/api\/sources\/([^/]+)\/image$/);
  if (m) {
    send(res, 200, ONE_BY_ONE.png, 'image/png');
    return;
  }
  m = url.pathname.match(/^\/api\/encodings\/([^/]+)\/image$/);
  if (m) {
    const id = m[1];
    const codec = id.split('__')[1] ?? 'mozjpeg';
    const { buf, mime } = blobForCodec(codec);
    send(res, 200, buf, mime);
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
