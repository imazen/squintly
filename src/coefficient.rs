//! Coefficient client. Two impls: HTTP (against a coefficient viewer) and FS (direct
//! SplitStore reads). Coefficient is consulted for both the manifest of available
//! sources/encodings and for raw image bytes (which we proxy through to the browser).
//!
//! We do NOT write to coefficient. Squintly is a consumer.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceMeta {
    pub hash: String,
    pub width: u32,
    pub height: u32,
    pub size_bytes: u64,
    pub corpus: Option<String>,
    pub filename: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncodingMeta {
    pub id: String,
    pub source_hash: String,
    pub codec: String,
    pub quality: Option<f32>,
    pub effort: Option<f32>,
    pub bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Manifest {
    pub sources: Vec<SourceMeta>,
    pub encodings: Vec<EncodingMeta>,
}

impl Manifest {
    pub fn encodings_for(&self, source_hash: &str) -> Vec<&EncodingMeta> {
        self.encodings.iter().filter(|e| e.source_hash == source_hash).collect()
    }

    pub fn source(&self, hash: &str) -> Option<&SourceMeta> {
        self.sources.iter().find(|s| s.hash == hash)
    }

    pub fn encoding(&self, id: &str) -> Option<&EncodingMeta> {
        self.encodings.iter().find(|e| e.id == id)
    }
}

#[allow(async_fn_in_trait)]
pub trait Coefficient: Send + Sync {
    async fn refresh_manifest(&self) -> Result<Manifest>;
    async fn fetch_source_png(&self, hash: &str) -> Result<(Vec<u8>, &'static str)>;
    async fn fetch_encoding_blob(&self, id: &str) -> Result<(Vec<u8>, String)>;
}

/// Concrete enum so we don't need `dyn Coefficient` in shared state.
pub enum CoefficientSource {
    Http(HttpCoefficient),
    Fs(FsCoefficient),
}

impl CoefficientSource {
    pub async fn refresh_manifest(&self) -> Result<Manifest> {
        match self {
            Self::Http(c) => c.refresh_manifest().await,
            Self::Fs(c) => c.refresh_manifest().await,
        }
    }
    pub async fn fetch_source_png(&self, hash: &str) -> Result<(Vec<u8>, String)> {
        match self {
            Self::Http(c) => c.fetch_source_png(hash).await.map(|(b, m)| (b, m.to_string())),
            Self::Fs(c) => c.fetch_source_png(hash).await.map(|(b, m)| (b, m.to_string())),
        }
    }
    pub async fn fetch_encoding_blob(&self, id: &str) -> Result<(Vec<u8>, String)> {
        match self {
            Self::Http(c) => c.fetch_encoding_blob(id).await,
            Self::Fs(c) => c.fetch_encoding_blob(id).await,
        }
    }
}

// ---------- HTTP backend ----------

pub struct HttpCoefficient {
    base: url::Url,
    http: reqwest::Client,
}

impl HttpCoefficient {
    pub fn new(base_url: &str) -> Result<Self> {
        let base = url::Url::parse(base_url).with_context(|| format!("invalid url: {base_url}"))?;
        let http = reqwest::Client::builder()
            .user_agent(concat!("squintly/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { base, http })
    }

    fn url(&self, path: &str) -> Result<url::Url> {
        Ok(self.base.join(path)?)
    }
}

impl Coefficient for HttpCoefficient {
    async fn refresh_manifest(&self) -> Result<Manifest> {
        // Coefficient's /api/manifest schema is rich; we tolerate unknown fields.
        // Expected: { sources: [{hash, width, height, size_bytes, corpus, filename}],
        //             encodings: [{id, source_hash, codec_name|codec, quality, effort,
        //             encoded_size|bytes}] }
        let url = self.url("/api/manifest")?;
        let resp = self.http.get(url).send().await?.error_for_status()?;
        let raw: serde_json::Value = resp.json().await?;
        Ok(parse_manifest_json(raw))
    }

    async fn fetch_source_png(&self, hash: &str) -> Result<(Vec<u8>, &'static str)> {
        let url = self.url(&format!("/api/sources/{hash}/image"))?;
        let resp = self.http.get(url).send().await?.error_for_status()?;
        let bytes = resp.bytes().await?.to_vec();
        Ok((bytes, "image/png"))
    }

    async fn fetch_encoding_blob(&self, id: &str) -> Result<(Vec<u8>, String)> {
        let url = self.url(&format!("/api/encodings/{id}/image"))?;
        let resp = self.http.get(url).send().await?.error_for_status()?;
        let mime = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let bytes = resp.bytes().await?.to_vec();
        Ok((bytes, mime))
    }
}

fn parse_manifest_json(v: serde_json::Value) -> Manifest {
    let mut m = Manifest::default();
    if let Some(arr) = v.get("sources").and_then(|x| x.as_array()) {
        for s in arr {
            if let Some(hash) = s.get("hash").and_then(|x| x.as_str()) {
                m.sources.push(SourceMeta {
                    hash: hash.to_string(),
                    width: s.get("width").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                    height: s.get("height").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                    size_bytes: s.get("size_bytes").and_then(|x| x.as_u64()).unwrap_or(0),
                    corpus: s.get("corpus").and_then(|x| x.as_str()).map(str::to_string),
                    filename: s.get("filename").and_then(|x| x.as_str()).map(str::to_string),
                });
            }
        }
    }
    if let Some(arr) = v.get("encodings").and_then(|x| x.as_array()) {
        for e in arr {
            let id = e.get("id").and_then(|x| x.as_str()).map(str::to_string);
            let source_hash = e.get("source_hash").and_then(|x| x.as_str()).map(str::to_string);
            if let (Some(id), Some(source_hash)) = (id, source_hash) {
                m.encodings.push(EncodingMeta {
                    id,
                    source_hash,
                    codec: e
                        .get("codec_name")
                        .or_else(|| e.get("codec"))
                        .and_then(|x| x.as_str())
                        .unwrap_or("unknown")
                        .to_string(),
                    quality: e.get("quality").and_then(|x| x.as_f64()).map(|v| v as f32),
                    effort: e.get("effort").and_then(|x| x.as_f64()).map(|v| v as f32),
                    bytes: e
                        .get("encoded_size")
                        .or_else(|| e.get("bytes"))
                        .and_then(|x| x.as_u64())
                        .unwrap_or(0),
                });
            }
        }
    }
    m
}

// ---------- Filesystem backend ----------

pub struct FsCoefficient {
    root: PathBuf,
}

impl FsCoefficient {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn meta_path(&self) -> PathBuf {
        self.root.join("meta")
    }
    fn blobs_path(&self) -> PathBuf {
        self.root.join("blobs")
    }

    /// Walk meta/ for `*.json` files, classify each by shape (source vs encoding).
    /// Coefficient's actual file naming is loose; we do best-effort.
    fn walk_meta(&self) -> Result<Vec<(PathBuf, serde_json::Value)>> {
        let mut out = Vec::new();
        let meta = self.meta_path();
        if !meta.exists() {
            return Ok(out);
        }
        walk(&meta, &mut out)?;
        Ok(out)
    }
}

fn walk(dir: &Path, out: &mut Vec<(PathBuf, serde_json::Value)>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_dir() {
            walk(&p, out)?;
        } else if p.extension().and_then(|s| s.to_str()) == Some("json") {
            if let Ok(text) = std::fs::read_to_string(&p) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                    out.push((p, v));
                }
            }
        }
    }
    Ok(())
}

impl Coefficient for FsCoefficient {
    async fn refresh_manifest(&self) -> Result<Manifest> {
        let entries = self.walk_meta()?;
        let mut m = Manifest::default();
        for (_path, v) in entries {
            // Heuristic: presence of `width`/`height` and absence of `source_hash`
            // ⇒ source; presence of `source_hash` ⇒ encoding.
            if v.get("source_hash").is_some() {
                if let Some(id) = v.get("id").and_then(|x| x.as_str()) {
                    m.encodings.push(EncodingMeta {
                        id: id.to_string(),
                        source_hash: v.get("source_hash").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                        codec: v
                            .get("codec_name")
                            .or_else(|| v.get("codec"))
                            .and_then(|x| x.as_str())
                            .unwrap_or("unknown")
                            .to_string(),
                        quality: v.get("quality").and_then(|x| x.as_f64()).map(|v| v as f32),
                        effort: v.get("effort").and_then(|x| x.as_f64()).map(|v| v as f32),
                        bytes: v
                            .get("encoded_size")
                            .or_else(|| v.get("bytes"))
                            .and_then(|x| x.as_u64())
                            .unwrap_or(0),
                    });
                }
            } else if let Some(hash) = v.get("hash").and_then(|x| x.as_str()) {
                m.sources.push(SourceMeta {
                    hash: hash.to_string(),
                    width: v.get("width").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                    height: v.get("height").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                    size_bytes: v.get("size_bytes").and_then(|x| x.as_u64()).unwrap_or(0),
                    corpus: v.get("corpus").and_then(|x| x.as_str()).map(str::to_string),
                    filename: v.get("filename").and_then(|x| x.as_str()).map(str::to_string),
                });
            }
        }
        Ok(m)
    }

    async fn fetch_source_png(&self, hash: &str) -> Result<(Vec<u8>, &'static str)> {
        // Coefficient stores source PNGs in blobs/sources/<hash>.png, fall back to
        // searching by prefix.
        let blobs = self.blobs_path();
        let direct = blobs.join("sources").join(format!("{hash}.png"));
        let chosen = if direct.exists() {
            direct
        } else {
            // 2-char prefix sharding fallback
            let sharded = blobs.join("sources").join(&hash[..2]).join(format!("{hash}.png"));
            if sharded.exists() {
                sharded
            } else {
                anyhow::bail!("source PNG for {hash} not found under {}", blobs.display());
            }
        };
        let bytes = tokio::fs::read(&chosen)
            .await
            .with_context(|| format!("reading {}", chosen.display()))?;
        Ok((bytes, "image/png"))
    }

    async fn fetch_encoding_blob(&self, id: &str) -> Result<(Vec<u8>, String)> {
        // Encoding blobs live in blobs/encodings/<id>.<ext>; we try common extensions.
        let dir = self.blobs_path().join("encodings");
        for ext in &["jpg", "jpeg", "webp", "avif", "jxl", "png"] {
            let p = dir.join(format!("{id}.{ext}"));
            if p.exists() {
                let mime = match *ext {
                    "jpg" | "jpeg" => "image/jpeg",
                    "webp" => "image/webp",
                    "avif" => "image/avif",
                    "jxl" => "image/jxl",
                    "png" => "image/png",
                    _ => "application/octet-stream",
                };
                let bytes = tokio::fs::read(&p).await?;
                return Ok((bytes, mime.to_string()));
            }
        }
        anyhow::bail!("encoding blob {id} not found under {}", dir.display());
    }
}
