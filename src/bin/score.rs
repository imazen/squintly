//! Compute SSIMULACRA2 for every encoding in the store, and write the TSV that
//! `POST /api/admin/metrics` ingests.
//!
//! # Why this exists as a batch tool
//!
//! Squintly collects one side of a correlation — how people rank encodings —
//! and until something supplies the other side its headline question cannot be
//! answered at all. Nothing in the pipeline computed a metric: the corpus
//! builder encodes and uploads, and stops there.
//!
//! It is a separate binary rather than a server route because it fetches and
//! decodes the whole corpus — thousands of blobs, minutes of CPU — which is an
//! operator's batch job, not something a web request should ever do.
//!
//! # What it does NOT do
//!
//! It never writes to the store. Squintly is a consumer of coefficient's images
//! and this is no exception: it reads blobs, computes numbers, and prints a
//! table. Ingest is a separate, explicit step against a running server, so the
//! scores can be inspected before they are joined to anybody's judgements.
//!
//! ```text
//! cargo run --release --bin squintly-score -- --out ~/tmp/ssim2.tsv
//! curl -X POST 'https://…/api/admin/metrics?source=ssim2-2026-08-06&format=tsv' \
//!      --data-binary @~/tmp/ssim2.tsv
//! ```

use anyhow::{Context, Result};
use clap::Parser;
use futures_util::StreamExt;
use imgref::ImgVec;
use squintly::coefficient::{CoefficientSource, HttpCoefficient};
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Parser)]
#[command(about = "Score every encoding in the store against its source with SSIMULACRA2")]
struct Args {
    /// Coefficient HTTP base, same value the server uses.
    #[arg(long, env = "SQUINTLY_COEFFICIENT_HTTP")]
    coefficient_http: String,
    /// Where to write the TSV. `-` for stdout.
    #[arg(long, default_value = "-")]
    out: String,
    /// How many sources to score concurrently. Each holds a decoded source and
    /// one decoded encoding in memory, and an XL source is ~9.5 MB compressed
    /// and far larger decoded, so this is deliberately modest.
    #[arg(long, default_value_t = 4)]
    concurrency: usize,
    /// Stop after this many sources. For a smoke run before committing to the
    /// whole corpus, which takes a while.
    #[arg(long)]
    limit: Option<usize>,
}

/// Decode any of the formats the corpus holds into tightly packed RGB8.
///
/// Alpha is composited onto white rather than dropped. SSIMULACRA2 takes RGB,
/// and discarding alpha would compare the *undefined* colour of fully
/// transparent pixels — which encoders are free to fill with anything, so the
/// score would measure the encoder's choice of invisible pixels rather than
/// anything a person could see.
fn decode_rgb8(bytes: &[u8], content_type: &str) -> Result<(Vec<u8>, usize, usize)> {
    let ct = content_type.to_ascii_lowercase();
    if ct.contains("png") || bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder.read_info().context("png header")?;
        let mut buf = vec![0u8; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).context("png frame")?;
        let (w, h) = (info.width as usize, info.height as usize);
        let rgb = match info.color_type {
            png::ColorType::Rgb => buf[..w * h * 3].to_vec(),
            png::ColorType::Rgba => composite_rgba(&buf, w, h),
            png::ColorType::Grayscale => buf[..w * h].iter().flat_map(|&g| [g, g, g]).collect(),
            png::ColorType::GrayscaleAlpha => {
                let mut out = Vec::with_capacity(w * h * 3);
                for px in buf[..w * h * 2].chunks_exact(2) {
                    let (g, a) = (px[0] as u32, px[1] as u32);
                    let v = ((g * a + 255 * (255 - a)) / 255) as u8;
                    out.extend_from_slice(&[v, v, v]);
                }
                out
            }
            png::ColorType::Indexed => {
                anyhow::bail!("indexed png should be expanded by the decoder")
            }
        };
        return Ok((rgb, w, h));
    }
    if ct.contains("jpeg") || ct.contains("jpg") || bytes.starts_with(&[0xFF, 0xD8]) {
        let mut d = jpeg_decoder::Decoder::new(std::io::Cursor::new(bytes));
        let pixels = d.decode().context("jpeg decode")?;
        let info = d.info().context("jpeg info")?;
        let (w, h) = (info.width as usize, info.height as usize);
        let rgb = match info.pixel_format {
            jpeg_decoder::PixelFormat::RGB24 => pixels,
            jpeg_decoder::PixelFormat::L8 => pixels.iter().flat_map(|&g| [g, g, g]).collect(),
            other => anyhow::bail!("unsupported jpeg pixel format {other:?}"),
        };
        return Ok((rgb, w, h));
    }
    if ct.contains("webp") || bytes.get(8..12) == Some(b"WEBP") {
        let mut d = image_webp::WebPDecoder::new(std::io::Cursor::new(bytes)).context("webp")?;
        let (w, h) = d.dimensions();
        let (w, h) = (w as usize, h as usize);
        let mut buf = vec![0u8; d.output_buffer_size().context("webp size")?];
        d.read_image(&mut buf).context("webp decode")?;
        let rgb = if d.has_alpha() {
            composite_rgba(&buf, w, h)
        } else {
            buf
        };
        return Ok((rgb, w, h));
    }
    if ct.contains("avif") || bytes.get(4..12) == Some(b"ftypavif") {
        // zenavif hands back a strided PixelBuffer, so rows must be repacked —
        // the first `width * channels` bytes of each `stride`-byte row are the
        // pixels. Reading it as tightly packed would shear the image on any
        // width the decoder padded, and a sheared reference scores every
        // encoding of that source as garbage.
        let buf = zenavif::decode(bytes).map_err(|e| anyhow::anyhow!("avif: {e}"))?;
        let (w, h) = (buf.width() as usize, buf.height() as usize);
        let channels = buf.descriptor().channels();
        let stride = buf.stride();
        let raw = buf.into_vec();
        let row_bytes = w * channels;
        let mut packed = Vec::with_capacity(w * h * channels);
        for row in 0..h {
            let off = row * stride;
            anyhow::ensure!(
                off + row_bytes <= raw.len(),
                "avif buffer short at row {row}"
            );
            packed.extend_from_slice(&raw[off..off + row_bytes]);
        }
        let rgb = match channels {
            3 => packed,
            4 => composite_rgba(&packed, w, h),
            n => anyhow::bail!("avif with {n} channels"),
        };
        return Ok((rgb, w, h));
    }
    anyhow::bail!("unrecognised content type `{content_type}`")
}

/// Composite RGBA over white.
fn composite_rgba(buf: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h * 3);
    for px in buf[..w * h * 4].chunks_exact(4) {
        let a = px[3] as u32;
        for &ch in &px[..3] {
            out.push(((ch as u32 * a + 255 * (255 - a)) / 255) as u8);
        }
    }
    out
}

fn to_img(rgb: &[u8], w: usize, h: usize) -> ImgVec<[u8; 3]> {
    ImgVec::new(
        rgb.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect(),
        w,
        h,
    )
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    tracing_subscriber::fmt().with_target(false).init();

    let coeff = Arc::new(CoefficientSource::Http(
        HttpCoefficient::new(&args.coefficient_http).context("coefficient base url")?,
    ));
    let manifest = coeff.refresh_manifest().await.context("fetch manifest")?;
    let mut sources = manifest.sources.clone();
    sources.sort_by(|a, b| a.hash.cmp(&b.hash));
    if let Some(n) = args.limit {
        sources.truncate(n);
    }
    tracing::info!(
        sources = sources.len(),
        encodings = manifest.encodings.len(),
        "scoring"
    );

    let done = Arc::new(AtomicUsize::new(0));
    let total = sources.len();
    let manifest = Arc::new(manifest);

    // Per SOURCE, not per encoding: the source is decoded once and reused for
    // all 24 of its encodings. Decoding an XL PNG per encoding instead would be
    // 24x the work for identical pixels.
    let results = futures_util::stream::iter(sources.into_iter().map(|src| {
        let coeff = Arc::clone(&coeff);
        let manifest = Arc::clone(&manifest);
        let done = Arc::clone(&done);
        async move {
            let out = score_source(&coeff, &manifest, &src.hash).await;
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            if n % 10 == 0 || n == total {
                tracing::info!("{n}/{total} sources");
            }
            match out {
                Ok(rows) => rows,
                Err(e) => {
                    // One unreadable source must not lose the other 167. The
                    // gap is visible in the output as a missing encoding id,
                    // which the ingest reports as reduced coverage rather than
                    // silently scoring it zero.
                    tracing::warn!(source = %src.hash, error = %e, "skipped");
                    Vec::new()
                }
            }
        }
    }))
    .buffer_unordered(args.concurrency)
    .collect::<Vec<_>>()
    .await;

    let mut rows: Vec<(String, f64)> = results.into_iter().flatten().collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    let mut sink: Box<dyn Write> = if args.out == "-" {
        Box::new(std::io::stdout())
    } else {
        Box::new(std::fs::File::create(&args.out).context("create --out")?)
    };
    writeln!(sink, "encoding_id\tssim2")?;
    for (id, score) in &rows {
        writeln!(sink, "{id}\t{score:.4}")?;
    }
    sink.flush()?;
    tracing::info!(rows = rows.len(), out = %args.out, "wrote");
    Ok(())
}

/// Score every encoding of one source.
async fn score_source(
    coeff: &CoefficientSource,
    manifest: &squintly::coefficient::Manifest,
    hash: &str,
) -> Result<Vec<(String, f64)>> {
    let (bytes, ct) = coeff.fetch_source_png(hash).await?;
    let (src_rgb, sw, sh) = decode_rgb8(&bytes, &ct).context("decode source")?;
    let src_img = to_img(&src_rgb, sw, sh);

    let mut out = Vec::new();
    for enc in manifest.encodings_for(hash) {
        let scored = async {
            let (eb, ect) = coeff.fetch_encoding_blob(&enc.id).await?;
            let (rgb, w, h) = decode_rgb8(&eb, &ect).context("decode encoding")?;
            // A size mismatch means the pair is not comparable — the metric
            // would either panic or silently score a resize. Refusing names the
            // encoding, which is exactly the corruption the identifier panel
            // exists to chase.
            anyhow::ensure!(
                w == sw && h == sh,
                "size mismatch: source {sw}x{sh}, encoding {w}x{h}"
            );
            let score =
                fast_ssim2::compute_ssimulacra2(src_img.as_ref(), to_img(&rgb, w, h).as_ref())
                    .map_err(|e| anyhow::anyhow!("ssimulacra2: {e:?}"))?;
            Ok::<f64, anyhow::Error>(score)
        }
        .await;
        match scored {
            Ok(s) => out.push((enc.id.clone(), s)),
            Err(e) => tracing::warn!(encoding = %enc.id, error = %e, "skipped"),
        }
    }
    Ok(out)
}
