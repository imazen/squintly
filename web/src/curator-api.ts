// Wraps /api/curator/* routes. Browser-side mirror of `src/curator.rs`.

export interface LicensePolicy {
  id: string;
  label: string;
  spdx_or_status: string;
  summary: string;
  terms_url: string;
  redistribute_bytes: boolean;
  commercial_training: boolean;
  attribution_required: boolean;
}

export interface Candidate {
  sha256: string;
  corpus: string;
  relative_path: string | null;
  width: number | null;
  height: number | null;
  size_bytes: number | null;
  format: string | null;
  suspected_category: string | null;
  has_alpha: boolean;
  has_animation: boolean;
  license_id: string;
  license_url: string | null;
  blob_url: string;
  order_hint: number;
  /// libjpeg-style q estimate (1..100) for JPEG sources, populated by the
  /// admin backfill. Null for non-JPEG or rows not yet processed.
  source_q_detected: number | null;
}

export interface Suggestion {
  groups: string[];
  sizes: number[];
  recommended_max_dim: number;
}

export type BppVerdict = 'Unknown' | 'Ok' | 'Low' | 'High';

export interface BppGate {
  bpp: number | null;
  verdict: BppVerdict;
  message: string;
}

export interface StreamResp {
  candidate: Candidate | null;
  license: LicensePolicy | null;
  suggestion: Suggestion | null;
  bpp_gate: BppGate | null;
  remaining: number;
  total: number;
}

export interface DecisionGroups {
  core_zensim?: boolean;
  medium_zensim?: boolean;
  full_zensim?: boolean;
  core_encoding?: boolean;
  medium_encoding?: boolean;
  full_encoding?: boolean;
}

export interface DecisionReq {
  source_sha256: string;
  curator_id: string;
  decision: 'take' | 'reject' | 'flag';
  reject_reason?: string | null;
  groups?: DecisionGroups;
  sizes?: number[];
  source_q_detected?: number | null;
  recommended_max_dim?: number | null;
  source_codec?: string | null;
  decision_dpr?: number | null;
  decision_viewport_w?: number | null;
  decision_viewport_h?: number | null;
}

export interface DecisionResp {
  decision_id: number;
  took: boolean;
}

export interface ThresholdReq {
  decision_id: number;
  target_max_dim: number;
  q_imperceptible: number;
  measurement_dpr: number;
  measurement_distance_cm?: number | null;
  encoder_label?: string;
}

export interface CorpusProgress {
  corpus: string;
  total: number;
  decided: number;
}

export interface ProgressResp {
  total_candidates: number;
  decisions: number;
  takes: number;
  rejects: number;
  flags: number;
  thresholds: number;
  by_corpus: CorpusProgress[];
}

export interface LoadManifestReq {
  kind: 'tsv' | 'jsonl';
  body: string;
  blob_url_base: string;
}

export interface LoadManifestResp {
  inserted: number;
  total: number;
}

async function jsonGet<T>(url: string): Promise<T> {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`${url} → ${r.status}`);
  return r.json() as Promise<T>;
}

async function jsonPost<TReq, TResp>(url: string, body: TReq): Promise<TResp> {
  const r = await fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });
  if (!r.ok) {
    const t = await r.text();
    throw new Error(t || `${url} → ${r.status}`);
  }
  return r.json() as Promise<TResp>;
}

export interface StreamFilter {
  skip?: number;
  source_q_detected?: number;
  corpus?: string[];
  license_id?: string[];
}

export function streamNext(curator_id: string, opts: StreamFilter = {}): Promise<StreamResp> {
  const u = new URL('/api/curator/stream/next', location.origin);
  u.searchParams.set('curator_id', curator_id);
  if (opts.skip != null) u.searchParams.set('skip', String(opts.skip));
  if (opts.source_q_detected != null) u.searchParams.set('source_q_detected', String(opts.source_q_detected));
  if (opts.corpus && opts.corpus.length > 0) u.searchParams.set('corpus', opts.corpus.join(','));
  if (opts.license_id && opts.license_id.length > 0) u.searchParams.set('license_id', opts.license_id.join(','));
  return jsonGet<StreamResp>(u.pathname + u.search);
}

export function postDecision(req: DecisionReq): Promise<DecisionResp> {
  return jsonPost('/api/curator/decision', req);
}

export interface UndoResp {
  undone: boolean;
  source_sha256: string | null;
  had_threshold: boolean;
}

export function undoDecision(curator_id: string, source_sha256?: string): Promise<UndoResp> {
  return jsonPost('/api/curator/decision/undo', { curator_id, source_sha256 });
}

export function postThreshold(req: ThresholdReq): Promise<{ ok: boolean }> {
  return jsonPost('/api/curator/threshold', req);
}

export interface GenerateVariantReq {
  decision_id: number;
  target_max_dim: number;
  /** "png" (default — lossless, training-safe) or "jpeg" (preview only). */
  format?: 'png' | 'jpeg';
  /** Only honored when format==='jpeg'. */
  quality?: number;
}

export interface GenerateVariantResp {
  ok: boolean;
  generated_sha256: string;
  generated_url: string;
  width: number;
  height: number;
  size_bytes: number;
  /** "png-rgba8-lossless" or "jpeg-qNN". */
  encoder_label: string;
}

export function generateVariant(req: GenerateVariantReq): Promise<GenerateVariantResp> {
  return jsonPost('/api/curator/generate-variant', req);
}

export function getProgress(curator_id: string): Promise<ProgressResp> {
  return jsonGet<ProgressResp>(`/api/curator/progress?curator_id=${encodeURIComponent(curator_id)}`);
}

export function loadManifest(req: LoadManifestReq): Promise<LoadManifestResp> {
  return jsonPost('/api/curator/manifest', req);
}

export function listLicenses(): Promise<LicensePolicy[]> {
  return jsonGet<LicensePolicy[]>('/api/curator/licenses');
}

const CURATOR_ID_KEY = 'squintly:curator_id';

export function getCuratorId(): string {
  let id = localStorage.getItem(CURATOR_ID_KEY);
  if (!id) {
    id = crypto.randomUUID();
    localStorage.setItem(CURATOR_ID_KEY, id);
  }
  return id;
}
