// Browser-side JPEG encoder used by the threshold slider.
//
// Note: this is `canvas.convertToBlob({type:'image/jpeg', quality})`, NOT
// jpegli. The browser's built-in JPEG encoder is a stand-in until we ship a
// WASM jpegli build. We document the encoder identity in the saved threshold
// row (`encoder_label = 'browser-canvas-jpeg'`) so downstream consumers can
// distinguish curator measurements taken with different encoders.
//
// Despite not being jpegli, the slider workflow is still useful: the curator
// is calibrating *their own* perception against a fixed encoder; the threshold
// values stay self-consistent within a corpus pass and shift uniformly when
// the encoder changes — which we record.

const ENCODE_QS = [30, 50, 70, 85, 95];

export interface EncodedSnapshot {
  q: number;
  blob: Blob;
  url: string;
}

/**
 * Decode an image (URL or already-loaded HTMLImageElement) onto a canvas, then
 * encode at five anchor q values in parallel. Returns the snapshots indexed by q.
 */
export async function preEncodeAnchors(source: HTMLImageElement): Promise<EncodedSnapshot[]> {
  const w = source.naturalWidth;
  const h = source.naturalHeight;
  if (!w || !h) throw new Error('source image has zero dimensions');
  const canvas = makeCanvas(w, h);
  const ctx = canvas.getContext('2d')!;
  ctx.drawImage(source, 0, 0);
  const out = await Promise.all(
    ENCODE_QS.map(async (q) => {
      const blob = await canvasToBlob(canvas, q);
      return { q, blob, url: URL.createObjectURL(blob) };
    }),
  );
  return out;
}

/**
 * Encode the source image at a single q value. Used by the slider for JIT
 * encodes between anchors.
 */
export async function encodeAtQ(source: HTMLImageElement, q: number): Promise<EncodedSnapshot> {
  const w = source.naturalWidth;
  const h = source.naturalHeight;
  const canvas = makeCanvas(w, h);
  canvas.getContext('2d')!.drawImage(source, 0, 0);
  const blob = await canvasToBlob(canvas, q);
  return { q, blob, url: URL.createObjectURL(blob) };
}

export function disposeSnapshots(snapshots: EncodedSnapshot[]): void {
  for (const s of snapshots) URL.revokeObjectURL(s.url);
}

function makeCanvas(w: number, h: number): HTMLCanvasElement | OffscreenCanvas {
  if (typeof OffscreenCanvas !== 'undefined') {
    return new OffscreenCanvas(w, h) as unknown as HTMLCanvasElement;
  }
  const c = document.createElement('canvas');
  c.width = w; c.height = h;
  return c;
}

function canvasToBlob(canvas: HTMLCanvasElement | OffscreenCanvas, q: number): Promise<Blob> {
  const quality = Math.max(0, Math.min(100, q)) / 100;
  if ((canvas as OffscreenCanvas).convertToBlob) {
    return (canvas as OffscreenCanvas).convertToBlob({ type: 'image/jpeg', quality });
  }
  return new Promise<Blob>((resolve, reject) => {
    (canvas as HTMLCanvasElement).toBlob(
      (b) => (b ? resolve(b) : reject(new Error('toBlob returned null'))),
      'image/jpeg',
      quality,
    );
  });
}

/** Anchors used by the slider. Exported for tests. */
export const ANCHOR_QS = ENCODE_QS;
