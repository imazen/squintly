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
}

export interface CreateSessionResp {
  observer_id: string;
  session_id: string;
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

export interface ResponseReq {
  choice: string;
  dwell_ms: number;
  reveal_count: number;
  reveal_ms_total: number;
  zoom_used: boolean;
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
