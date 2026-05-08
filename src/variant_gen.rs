//! Variant generation pipeline.
//!
//! Decode a candidate's source bytes → resize via zenresize Mitchell-Netravali
//! → JPEG-encode at the curator's chosen quality → upload to R2 under
//! `variants/{xx}/{yy}/{sha}.jpg`. Spec §4 (`POST
//! /api/curator/generate-variant`).
//!
//! Decode side uses pure-Rust crates (`jpeg-decoder`, `png`, `image-webp`)
//! while the imazen ecosystem's `zenjpeg` / `zenpng` / `zenwebp` work through
//! the magetypes/archmage version skew on crates.io. The encode side uses
//! `jpeg-encoder` for the same reason. Resize is `zenresize::Resizer` with
//! `Filter::Mitchell` — the part of the pipeline where the imazen choice
//! actually matters perceptually for downstream training data.
//!
//! AVIF and JXL sources currently 415 — a follow-up swap to zenavif/zenjxl
//! when those stabilize on crates.io will close this gap. WebP is supported
//! via `image-webp`.

use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum VariantError {
    #[error("unsupported source format: {0}")]
    UnsupportedFormat(String),
    #[error("decode failed: {0}")]
    Decode(String),
    #[error("encode failed: {0}")]
    Encode(String),
    #[error("fetch source failed: {0}")]
    Fetch(String),
}

/// One generated variant ready to persist.
pub struct Variant {
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
    pub sha256: String,
    pub mime: &'static str,
}

/// Run the full pipeline for a single (source_bytes, target_max_dim, quality)
/// triple. Returns the encoded JPEG bytes, the new sha256, and the new dims.
///
/// `format_hint` is the candidate's stored format (`"jpeg"` etc.). When
/// `None`, format is sniffed from the bytes.
pub fn generate(
    source_bytes: &[u8],
    format_hint: Option<&str>,
    target_max_dim: u32,
    quality: u8,
) -> Result<Variant, VariantError> {
    let (rgba, w, h) = decode_to_rgba(source_bytes, format_hint)?;
    let (out_w, out_h) = fit_to_max(w, h, target_max_dim);
    let resized = if (out_w, out_h) == (w, h) {
        rgba
    } else {
        resize_rgba_mitchell(&rgba, w, h, out_w, out_h)?
    };
    let jpeg = encode_jpeg(&resized, out_w, out_h, quality)?;
    let mut hasher = Sha256::new();
    hasher.update(&jpeg);
    let sha = hex::encode(hasher.finalize());
    Ok(Variant {
        width: out_w,
        height: out_h,
        bytes: jpeg,
        sha256: sha,
        mime: "image/jpeg",
    })
}

fn fit_to_max(w: u32, h: u32, target_max: u32) -> (u32, u32) {
    let cur_max = w.max(h);
    if cur_max <= target_max {
        return (w, h);
    }
    let scale = target_max as f64 / cur_max as f64;
    let nw = ((w as f64) * scale).round().max(1.0) as u32;
    let nh = ((h as f64) * scale).round().max(1.0) as u32;
    (nw, nh)
}

// ---------- decode ----------

fn decode_to_rgba(
    bytes: &[u8],
    format_hint: Option<&str>,
) -> Result<(Vec<u8>, u32, u32), VariantError> {
    let fmt = sniff_format(bytes, format_hint);
    match fmt.as_deref() {
        Some("jpeg") => decode_jpeg(bytes),
        Some("png") => decode_png(bytes),
        Some("webp") => decode_webp(bytes),
        Some("avif") => decode_avif(bytes),
        Some(other) => Err(VariantError::UnsupportedFormat(other.to_string())),
        None => Err(VariantError::UnsupportedFormat("unknown".to_string())),
    }
}

fn sniff_format(bytes: &[u8], hint: Option<&str>) -> Option<String> {
    if let Some(h) = hint {
        let lower = h.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "jpeg" | "jpg" | "png" | "webp" | "avif" | "jxl" | "gif" | "heic"
        ) {
            return Some(if lower == "jpg" {
                "jpeg".to_string()
            } else {
                lower
            });
        }
    }
    if bytes.starts_with(b"\xff\xd8\xff") {
        return Some("jpeg".to_string());
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("png".to_string());
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("webp".to_string());
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        let brand = &bytes[8..12];
        if brand == b"avif" || brand == b"avis" {
            return Some("avif".to_string());
        }
        if brand == b"heic" {
            return Some("heic".to_string());
        }
    }
    None
}

fn decode_jpeg(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), VariantError> {
    let mut decoder = jpeg_decoder::Decoder::new(bytes);
    let pixels = decoder
        .decode()
        .map_err(|e| VariantError::Decode(format!("jpeg: {e}")))?;
    let info = decoder
        .info()
        .ok_or_else(|| VariantError::Decode("jpeg: no info".into()))?;
    let w = info.width as u32;
    let h = info.height as u32;
    let rgba = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => rgb_to_rgba(&pixels),
        jpeg_decoder::PixelFormat::L8 => luma_to_rgba(&pixels),
        jpeg_decoder::PixelFormat::CMYK32 => {
            return Err(VariantError::Decode("jpeg: CMYK not supported".into()));
        }
        jpeg_decoder::PixelFormat::L16 => {
            return Err(VariantError::Decode(
                "jpeg: 16-bit luma not supported".into(),
            ));
        }
    };
    Ok((rgba, w, h))
}

fn decode_png(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), VariantError> {
    let cursor = std::io::Cursor::new(bytes);
    let mut decoder = png::Decoder::new(cursor);
    decoder.set_transformations(png::Transformations::ALPHA | png::Transformations::EXPAND);
    let mut reader = decoder
        .read_info()
        .map_err(|e| VariantError::Decode(format!("png: {e}")))?;
    let info = reader.info().clone();
    let mut buf = vec![0u8; reader.output_buffer_size()];
    reader
        .next_frame(&mut buf)
        .map_err(|e| VariantError::Decode(format!("png: {e}")))?;
    let w = info.width;
    let h = info.height;
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => rgb_to_rgba(&buf),
        png::ColorType::GrayscaleAlpha => grayscale_alpha_to_rgba(&buf),
        png::ColorType::Grayscale => luma_to_rgba(&buf),
        png::ColorType::Indexed => {
            return Err(VariantError::Decode(
                "png: indexed should have been expanded".into(),
            ));
        }
    };
    Ok((rgba, w, h))
}

fn decode_webp(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), VariantError> {
    let mut decoder = image_webp::WebPDecoder::new(std::io::Cursor::new(bytes))
        .map_err(|e| VariantError::Decode(format!("webp: {e}")))?;
    let (w, h) = decoder.dimensions();
    let has_alpha = decoder.has_alpha();
    let mut buf = if has_alpha {
        vec![0u8; (w * h * 4) as usize]
    } else {
        vec![0u8; (w * h * 3) as usize]
    };
    decoder
        .read_image(&mut buf)
        .map_err(|e| VariantError::Decode(format!("webp: {e}")))?;
    let rgba = if has_alpha { buf } else { rgb_to_rgba(&buf) };
    Ok((rgba, w, h))
}

fn decode_avif(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), VariantError> {
    let buffer = zenavif::decode(bytes).map_err(|e| VariantError::Decode(format!("avif: {e}")))?;
    let w = buffer.width();
    let h = buffer.height();
    let descriptor = buffer.descriptor();
    let channels = descriptor.channels();
    let stride = buffer.stride();
    let raw = buffer.into_vec();
    // PixelBuffer rows are `stride` bytes each, but the first `width *
    // channels` bytes per row are pixel data. Repack into a tightly-packed
    // RGBA Vec<u8>.
    let row_bytes = (w as usize) * channels;
    let mut packed = Vec::with_capacity((w as usize) * (h as usize) * channels);
    for row in 0..(h as usize) {
        let off = row * stride;
        if off + row_bytes > raw.len() {
            return Err(VariantError::Decode(format!(
                "avif: short buffer (stride={stride} row={row} len={})",
                raw.len()
            )));
        }
        packed.extend_from_slice(&raw[off..off + row_bytes]);
    }
    let rgba = match channels {
        4 => packed,
        3 => rgb_to_rgba(&packed),
        1 => luma_to_rgba(&packed),
        n => {
            return Err(VariantError::Decode(format!(
                "avif: unexpected channel count {n}"
            )));
        }
    };
    Ok((rgba, w, h))
}

fn rgb_to_rgba(rgb: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgb.len() / 3 * 4);
    for chunk in rgb.chunks_exact(3) {
        out.extend_from_slice(chunk);
        out.push(0xff);
    }
    out
}

fn luma_to_rgba(l: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(l.len() * 4);
    for &v in l {
        out.push(v);
        out.push(v);
        out.push(v);
        out.push(0xff);
    }
    out
}

fn grayscale_alpha_to_rgba(la: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(la.len() * 2);
    for chunk in la.chunks_exact(2) {
        let v = chunk[0];
        let a = chunk[1];
        out.push(v);
        out.push(v);
        out.push(v);
        out.push(a);
    }
    out
}

// ---------- resize ----------

fn resize_rgba_mitchell(
    rgba: &[u8],
    in_w: u32,
    in_h: u32,
    out_w: u32,
    out_h: u32,
) -> Result<Vec<u8>, VariantError> {
    let config = zenresize::ResizeConfig::builder(in_w, in_h, out_w, out_h)
        .filter(zenresize::filter::Filter::Mitchell)
        .input(zenpixels::PixelDescriptor::RGBA8_SRGB)
        .build();
    let mut resizer = zenresize::Resizer::new(&config);
    Ok(resizer.resize(rgba))
}

// ---------- encode ----------

fn encode_jpeg(rgba: &[u8], w: u32, h: u32, quality: u8) -> Result<Vec<u8>, VariantError> {
    let mut out = Vec::with_capacity((w as usize * h as usize) / 4);
    let mut encoder = jpeg_encoder::Encoder::new(&mut out, quality);
    encoder.set_progressive(false);
    encoder
        .encode(rgba, w as u16, h as u16, jpeg_encoder::ColorType::Rgba)
        .map_err(|e| VariantError::Encode(format!("jpeg: {e}")))?;
    Ok(out)
}

// ---------- fetcher (used by handler) ----------

pub async fn fetch_source(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, VariantError> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| VariantError::Fetch(format!("{url}: {e}")))?
        .error_for_status()
        .map_err(|e| VariantError::Fetch(format!("HTTP: {e}")))?;
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| VariantError::Fetch(format!("read body: {e}")))?;
    Ok(bytes.to_vec())
}

// ---------- tests ----------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_to_max_no_op() {
        assert_eq!(fit_to_max(100, 50, 200), (100, 50));
    }

    #[test]
    fn fit_to_max_landscape() {
        // 2400x1800, max=512 → 512x384
        assert_eq!(fit_to_max(2400, 1800, 512), (512, 384));
    }

    #[test]
    fn fit_to_max_portrait() {
        // 1800x2400, max=512 → 384x512
        assert_eq!(fit_to_max(1800, 2400, 512), (384, 512));
    }

    #[test]
    fn rgb_to_rgba_pads_alpha() {
        let rgb = vec![10, 20, 30, 40, 50, 60];
        let rgba = rgb_to_rgba(&rgb);
        assert_eq!(rgba, vec![10, 20, 30, 0xff, 40, 50, 60, 0xff]);
    }

    #[test]
    fn sniff_jpeg() {
        let mut bytes = vec![0xff, 0xd8, 0xff, 0xe0];
        bytes.extend_from_slice(&[0u8; 32]);
        assert_eq!(sniff_format(&bytes, None).as_deref(), Some("jpeg"));
    }

    #[test]
    fn sniff_with_hint_normalizes_jpg() {
        assert_eq!(sniff_format(&[], Some("jpg")).as_deref(), Some("jpeg"));
        assert_eq!(sniff_format(&[], Some("WEBP")).as_deref(), Some("webp"));
    }

    #[test]
    fn full_pipeline_jpeg_roundtrip() {
        // Encode a tiny test image, then run it through generate() to confirm
        // the whole decode → resize → encode chain runs and produces JPEG.
        let mut src = Vec::new();
        let mut enc = jpeg_encoder::Encoder::new(&mut src, 90);
        enc.set_progressive(false);
        // 32x32 red square (RGBA)
        let pixels: Vec<u8> = (0..32 * 32).flat_map(|_| [200, 50, 50, 255]).collect();
        enc.encode(&pixels, 32, 32, jpeg_encoder::ColorType::Rgba)
            .unwrap();

        let v = generate(&src, Some("jpeg"), 16, 80).expect("generate");
        assert_eq!(v.width, 16);
        assert_eq!(v.height, 16);
        assert!(v.bytes.starts_with(b"\xff\xd8\xff"));
        assert_eq!(v.mime, "image/jpeg");
        assert_eq!(v.sha256.len(), 64);
    }
}
