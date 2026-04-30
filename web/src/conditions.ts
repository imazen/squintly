// Per-session and per-trial viewing-condition capture. Phone-first; assume nothing.

export interface SessionConditions {
  device_pixel_ratio: number;
  screen_width_css: number;
  screen_height_css: number;
  color_gamut: string;            // 'srgb' | 'p3' | 'rec2020' | 'unknown'
  dynamic_range_high: boolean;
  prefers_dark: boolean;
  pointer_type: string;           // 'fine' | 'coarse' | 'unknown'
  timezone: string;
  user_agent: string;
}

export function captureSession(): SessionConditions {
  const gamut = ['rec2020', 'p3', 'srgb'].find((g) =>
    matchMedia(`(color-gamut: ${g})`).matches,
  ) ?? 'unknown';
  const pointer = matchMedia('(pointer: coarse)').matches
    ? 'coarse'
    : matchMedia('(pointer: fine)').matches
    ? 'fine'
    : 'unknown';
  return {
    device_pixel_ratio: window.devicePixelRatio ?? 1,
    screen_width_css: screen.width,
    screen_height_css: screen.height,
    color_gamut: gamut,
    dynamic_range_high: matchMedia('(dynamic-range: high)').matches,
    prefers_dark: matchMedia('(prefers-color-scheme: dark)').matches,
    pointer_type: pointer,
    timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    user_agent: navigator.userAgent,
  };
}

export interface TrialConditions {
  viewport_w_css: number;
  viewport_h_css: number;
  orientation: 'portrait' | 'landscape';
  image_displayed_w_css: number;
  image_displayed_h_css: number;
  intrinsic_to_device_ratio: number;
  pixels_per_degree: number | null;
}

export function captureTrial(
  img: HTMLImageElement,
  cssPxPerMm: number | null,
  viewingDistanceCm: number | null,
): TrialConditions {
  const rect = img.getBoundingClientRect();
  const dpr = window.devicePixelRatio ?? 1;
  const intrinsicW = img.naturalWidth || 1;
  const deviceW = Math.max(1, rect.width * dpr);
  const ratio = intrinsicW / deviceW;
  let ppd: number | null = null;
  if (cssPxPerMm && viewingDistanceCm && viewingDistanceCm > 0) {
    // Device px per degree of visual angle:
    // device_px / mm = cssPxPerMm * dpr (since cssPxPerMm is CSS px per mm).
    // mm per degree ≈ viewing_distance_mm * tan(1°) ≈ viewing_distance_mm * 0.01745.
    const devicePxPerMm = cssPxPerMm * dpr;
    const mmPerDeg = (viewingDistanceCm * 10) * Math.tan((1 * Math.PI) / 180);
    ppd = devicePxPerMm * mmPerDeg;
  }
  return {
    viewport_w_css: window.innerWidth,
    viewport_h_css: window.innerHeight,
    orientation: matchMedia('(orientation: portrait)').matches ? 'portrait' : 'landscape',
    image_displayed_w_css: rect.width,
    image_displayed_h_css: rect.height,
    intrinsic_to_device_ratio: ratio,
    pixels_per_degree: ppd,
  };
}

const OBSERVER_KEY = 'squintly:observer_id';
const CALIB_KEY = 'squintly:calibration';
const PROFILE_KEY = 'squintly:profile';

export function getObserverId(): string | null {
  return localStorage.getItem(OBSERVER_KEY);
}
export function setObserverId(id: string): void {
  localStorage.setItem(OBSERVER_KEY, id);
}

export interface Calibration {
  css_px_per_mm: number | null;
  viewing_distance_cm: number | null;
}
export function loadCalibration(): Calibration {
  try {
    return JSON.parse(localStorage.getItem(CALIB_KEY) || 'null') ?? { css_px_per_mm: null, viewing_distance_cm: null };
  } catch {
    return { css_px_per_mm: null, viewing_distance_cm: null };
  }
}
export function saveCalibration(c: Calibration): void {
  localStorage.setItem(CALIB_KEY, JSON.stringify(c));
}

export interface Profile {
  age_bracket: string | null;
  vision_corrected: string | null;
  ambient_light: string | null;
}
export function loadProfile(): Profile {
  try {
    return JSON.parse(localStorage.getItem(PROFILE_KEY) || 'null') ?? { age_bracket: null, vision_corrected: null, ambient_light: null };
  } catch {
    return { age_bracket: null, vision_corrected: null, ambient_light: null };
  }
}
export function saveProfile(p: Profile): void {
  localStorage.setItem(PROFILE_KEY, JSON.stringify(p));
}
