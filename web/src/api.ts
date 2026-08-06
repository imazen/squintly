// Thin axum API client.

export interface CreateSessionReq {
  observer_id: string | null;
  user_agent: string;
  age_bracket: string | null;
  vision_corrected: string | null;
  device_pixel_ratio: number;
  screen_width_css: number;
  screen_height_css: number;
  color_gamut: string;
  dynamic_range_high: boolean;
  prefers_dark: boolean;
  pointer_type: string;
  timezone: string;
  viewing_distance_cm: number | null;
  ambient_light: string | null;
  css_px_per_mm: number | null;
  notes?: string;
  theme_slug?: string | null;
  local_date?: string | null;
  supported_codecs?: string[];
  codec_probe_cached?: boolean;
  /// Which named study to join (`GET /api/studies`). Omitted = the
  /// deployment's default. An unknown id is rejected, not coerced.
  study_id?: string | null;
}

export interface CreateSessionResp {
  observer_id: string;
  session_id: string;
  study_id: string;
  streak_days: number;
  streak_outcome: 'advanced' | 'frozen' | 'reset' | 'same_day' | 'skipped';
  freezes_remaining: number;
  total_trials: number;
}

export async function createSession(req: CreateSessionReq): Promise<CreateSessionResp> {
  const r = await fetch('/api/session', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(req),
  });
  if (!r.ok) throw new Error(`createSession ${r.status}`);
  return r.json();
}

export interface TrialEncoding {
  encoding_id: string;
  url: string;
  codec: string;
  quality: number | null;
  bytes: number;
}

export interface TrialPayload {
  trial_id: string;
  kind: 'single' | 'pair';
  source_hash: string;
  source_url: string;
  /// The store's own name for the image. A sha256 identifies it; the filename
  /// is what a person uses to find it again.
  source_filename: string | null;
  /// Header-width form of the filename — no extension, size rung or trailing
  /// dimensions.
  source_label: string | null;
  /// The part of the corpus name the filename does not already say; null when
  /// it says all of it.
  source_group: string | null;
  source_w: number;
  source_h: number;
  source_corpus: string | null;
  source_license_id: string;
  source_license_label: string;
  a: TrialEncoding;
  b: TrialEncoding | null;
  staircase_target: string | null;
}

export async function nextTrial(session_id: string): Promise<TrialPayload> {
  const u = `/api/trial/next?session_id=${encodeURIComponent(session_id)}`;
  const r = await fetch(u);
  if (!r.ok) throw new Error(`nextTrial ${r.status}`);
  return r.json();
}

export interface Study {
  id: string;
  label: string;
  /// Two words, for the corner of the trial screen. The full `label` is a
  /// sentence — fine in a picker, useless in the one line of chrome above a
  /// stimulus on a phone.
  short_name: string;
  is_default: boolean;
  summary: string;
  trial_style: string;
  unlisted: boolean;
}

export async function listStudies(): Promise<Study[]> {
  const r = await fetch('/api/studies');
  if (!r.ok) throw new Error(`listStudies ${r.status}`);
  return r.json();
}

export interface ResponseReq {
  choice: string;
  dwell_ms: number;
  reveal_count: number;
  reveal_ms_total: number;
  zoom_used: boolean;
  /// Panning, recorded because the stimulus is displayed at a hard minimum of
  /// 1:1 device pixels — anything bigger than the screen is only partly
  /// visible, so `image_displayed_*` no longer describes what was looked at.
  pan_count: number;
  pan_distance_css: number;
  /// Magnification at response time; 1 = native 1:1, integers only.
  zoom_factor: number;
  pannable_w_css: number;
  pannable_h_css: number;
  visible_w_css: number;
  visible_h_css: number;
  /// How the observer drove the UI. Changes what `reveal_ms_total` measures
  /// (see migration 0017), so it travels with every response rather than being
  /// inferred later.
  input_mode: 'tap' | 'hold' | 'buttons';
  keyboard_used: boolean;
  /// Render → judged-image-painted, kept out of `dwell_ms`'s interpretation.
  ui_ready_ms: number | null;
  /// Difficulty signal: how often the observer swapped view, and how long each
  /// variant was actually on screen. Raw, not normalised — the useful form is
  /// relative to their other trials this session, which is not knowable yet.
  switch_count: number;
  ms_on_a: number;
  ms_on_b: number;
  ms_on_ref: number;
  /// ms into the trial when the UI suggested "can't tell"; null if it never
  /// did. A nudge toward one answer has to be conditionable in analysis.
  cant_tell_hint_ms: number | null;
  /// How many process nudges this observer had seen in this session before
  /// answering. Answer-neutral, so it cannot bias `choice` — but it changes
  /// `switch_count` and `dwell_ms` on later trials, which is what effort is
  /// read from. See `nudge.ts` and migration 0024.
  process_nudges_seen: number;
  viewport_w_css: number;
  viewport_h_css: number;
  orientation: 'portrait' | 'landscape';
  image_displayed_w_css: number;
  image_displayed_h_css: number;
  intrinsic_to_device_ratio: number;
  pixels_per_degree: number | null;
}

export interface ResponseAck {
  total_trials: number;
  milestone_badge: string | null;
  flags: string | null;
  /// Lifetime comparisons by this observer — server-side, so it survives
  /// sessions and devices. The lap bar is drawn from it.
  total_comparisons: number;
  /// Comparisons per lap, from the server so the threshold lives in one place.
  comparisons_per_lap: number;
}

export async function recordResponse(
  trial_id: string,
  body: ResponseReq,
): Promise<ResponseAck | null> {
  const r = await fetch(`/api/trial/${encodeURIComponent(trial_id)}/response`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });
  if (!r.ok) throw new Error(`recordResponse ${r.status}`);
  return (await r.json().catch(() => null)) as ResponseAck | null;
}

export async function endSession(session_id: string): Promise<void> {
  await fetch(`/api/session/${encodeURIComponent(session_id)}/end`, { method: 'POST' });
}

export interface AuthStartReq {
  email: string;
  observer_id: string | null;
  origin: string;
}

export interface AuthStartResp {
  ok: boolean;
  message: string;
}

export async function authStart(body: AuthStartReq): Promise<AuthStartResp> {
  const r = await fetch('/api/auth/start', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });
  if (!r.ok) {
    const text = await r.text();
    throw new Error(text || `authStart ${r.status}`);
  }
  return r.json();
}

export interface LeaderboardRow {
  handle: string;
  /// The caller's own row, when `listLeaderboard` was given an observer id.
  is_you: boolean;
  trials: number;
  sessions: number;
  active_days: number;
  /// Attention-check pass rate. `null` when none have been served yet — which
  /// is NOT the same as 0, a failing reviewer.
  golden_pass_rate: number | null;
  /// Agreement with themselves on re-served pairs: the ceiling any metric
  /// could reach against this reviewer. High volume with low self-agreement is
  /// noise, not data, which is why it sits beside the trial count.
  self_agreement: number | null;
  repeat_pairs: number;
  median_seconds: number | null;
  /// Over instrumented responses only — a row written before migration 0019
  /// carries the column's DEFAULT 0, which is "not recorded", not "did not
  /// switch". Read against `instrumented_trials`.
  median_switches: number | null;
  instrumented_trials: number;
  /// Engaged time: gaps between consecutive answers in a session, each capped
  /// at the server's idle cap, plus the first answer's dwell. Reproducible from
  /// responses.tsv, so the figure can be checked rather than trusted.
  active_seconds: number;
}

export async function listLeaderboard(observerId?: string | null): Promise<LeaderboardRow[]> {
  const q = observerId ? `?observer_id=${encodeURIComponent(observerId)}` : '';
  const r = await fetch(`/api/leaderboard${q}`);
  if (!r.ok) throw new Error(`leaderboard ${r.status}`);
  return r.json();
}

export interface WhoAmI {
  signed_in: boolean;
  email: string | null;
  observer_id: string | null;
  is_admin: boolean;
}

export async function whoami(): Promise<WhoAmI> {
  const r = await fetch('/api/auth/whoami');
  if (!r.ok) return { signed_in: false, email: null, observer_id: null, is_admin: false };
  return r.json();
}

export async function signOut(): Promise<void> {
  await fetch('/api/auth/signout', { method: 'POST' });
}

export interface StudyProgress {
  id: string;
  short_name: string;
  label: string;
  is_default: boolean;
  responses: number;
  min_viable_ratings: number;
  ideal_ratings: number;
  observers: number;
}

export async function studyProgress(): Promise<StudyProgress[]> {
  const r = await fetch('/api/studies/progress');
  if (!r.ok) throw new Error(`studyProgress ${r.status}`);
  return r.json();
}

/// One metric's agreement with the observers, from `/api/admin/disposition`.
export interface MetricAgreement {
  metric: string;
  direction: 'higher_is_better' | 'lower_is_better' | 'unknown';
  comparisons: number;
  agreed: number;
  /// null below the minimum sample — deliberately not 0.
  rho: number | null;
  /// The reportable figure. null whenever rho or the ceiling is.
  rho_over_ceiling: number | null;
  ties: number;
  uncovered: number;
}

export interface NoiseCeiling {
  repeat_pairs: number;
  agreed: number;
  ceiling: number | null;
}

export interface Disposition {
  study_id: string;
  comparisons: number;
  distinct_pairs: number;
  observers: number;
  min_viable_ratings: number;
  ideal_ratings: number;
  ceiling: NoiseCeiling;
  golden_pass_rate: number | null;
  golden_trials: number;
  metrics: MetricAgreement[];
  unusable: { metric: string; reason: string }[];
}

export async function disposition(studyId?: string): Promise<Disposition> {
  const q = studyId ? `?study_id=${encodeURIComponent(studyId)}` : '';
  const r = await fetch(`/api/admin/disposition${q}`);
  if (!r.ok) throw new Error(`disposition ${r.status}`);
  return r.json();
}

export interface MetricCatalogRow {
  metric: string;
  encodings: number;
  direction: 'higher_is_better' | 'lower_is_better' | 'unknown';
  blurb: string | null;
  min: number;
  max: number;
  covered_encodings: number;
}

export async function metricCatalog(): Promise<MetricCatalogRow[]> {
  const r = await fetch('/api/admin/metrics');
  if (!r.ok) throw new Error(`metricCatalog ${r.status}`);
  return r.json();
}

export interface EncodingMetricRow {
  encoding_id: string;
  metric: string;
  value: number;
}

/// Metric scores for specific encodings. Admin-only server-side.
///
/// Resolves to `[]` on any failure — including the 403 an ordinary observer
/// gets. The identifier panel then simply shows what it always showed, with no
/// gap where something was withheld and no error to explain. The gate is the
/// server's; this is only how the UI declines to make a fuss about it.
export async function encodingMetrics(ids: string[]): Promise<EncodingMetricRow[]> {
  if (!ids.length) return [];
  try {
    const r = await fetch(`/api/admin/metrics/encodings?ids=${encodeURIComponent(ids.join(','))}`);
    if (!r.ok) return [];
    return await r.json();
  } catch {
    return [];
  }
}
