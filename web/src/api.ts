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
  viewport_w_css: number;
  viewport_h_css: number;
  orientation: 'portrait' | 'landscape';
  image_displayed_w_css: number;
  image_displayed_h_css: number;
  intrinsic_to_device_ratio: number;
  pixels_per_degree: number | null;
}

export async function recordResponse(trial_id: string, body: ResponseReq): Promise<void> {
  const r = await fetch(`/api/trial/${encodeURIComponent(trial_id)}/response`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });
  if (!r.ok) throw new Error(`recordResponse ${r.status}`);
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
