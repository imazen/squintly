//! Storage backend for public suggestion uploads.
//!
//! Two impls:
//!   - `LocalDiskStore` (default in dev / tests): writes content-addressed
//!     blobs under `<base>/{xx}/{yy}/{sha}.{ext}` and serves bytes through
//!     the squintly process.
//!   - `R2Store` (production): PUTs blobs to a Cloudflare R2 / S3-compatible
//!     bucket under `suggestions/{xx}/{yy}/{sha}.{ext}` and returns the
//!     bucket's public URL so the browser fetches directly from R2.
//!
//! Selection happens at boot: when all four `SQUINTLY_R2_*` env vars are
//! set, R2 is used; otherwise we fall back to local disk. Production
//! deployments should always have R2 wired so suggestions land in the same
//! bucket as the rest of the corpus.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use rusty_s3::actions::{PutObject, S3Action};
use rusty_s3::{Bucket, Credentials, UrlStyle};

#[derive(Clone)]
pub enum SuggestionStore {
    LocalDisk(LocalDiskStore),
    R2(Arc<R2Store>),
}

impl SuggestionStore {
    /// Resolve from env. Returns the R2 backend when all four
    /// `SQUINTLY_R2_*` env vars are set; falls back to `LocalDisk(local_default)`.
    pub fn from_env(local_default: PathBuf) -> Self {
        match R2Store::from_env() {
            Ok(Some(r)) => {
                tracing::info!(bucket = %r.bucket_name(), "suggestion store: R2");
                SuggestionStore::R2(Arc::new(r))
            }
            Ok(None) => {
                tracing::info!(
                    path = %local_default.display(),
                    "suggestion store: local disk (set SQUINTLY_R2_* env vars to enable R2)"
                );
                SuggestionStore::LocalDisk(LocalDiskStore::new(local_default))
            }
            Err(e) => {
                tracing::warn!(error = %e, "R2 config invalid; falling back to local disk");
                SuggestionStore::LocalDisk(LocalDiskStore::new(local_default))
            }
        }
    }

    /// Description shown in `/api/stats` and logs.
    pub fn label(&self) -> String {
        match self {
            SuggestionStore::LocalDisk(s) => format!("local-disk:{}", s.base.display()),
            SuggestionStore::R2(r) => format!(
                "r2:{}{}",
                r.bucket.name(),
                r.public_base
                    .as_deref()
                    .map(|b| format!(" public={b}"))
                    .unwrap_or_default()
            ),
        }
    }

    /// Persist `bytes` for sha+ext under the default `suggestions/` prefix.
    pub async fn put(
        &self,
        sha: &str,
        ext: &str,
        bytes: &[u8],
        mime: &str,
    ) -> Result<StoredObject> {
        self.put_with_prefix("suggestions", sha, ext, bytes, mime)
            .await
    }

    /// Persist under a custom top-level prefix (e.g. `"variants"`). Same
    /// content-addressed `{xx}/{yy}/{sha}.{ext}` layout under the prefix.
    pub async fn put_with_prefix(
        &self,
        prefix: &str,
        sha: &str,
        ext: &str,
        bytes: &[u8],
        mime: &str,
    ) -> Result<StoredObject> {
        match self {
            SuggestionStore::LocalDisk(s) => s.put(prefix, sha, ext, bytes).await,
            SuggestionStore::R2(r) => r.put(prefix, sha, ext, bytes, mime).await,
        }
    }

    /// Read bytes back. Used by the proxy `/file` endpoint when no public
    /// URL is available. R2-backed uploads should normally be served via the
    /// public URL directly; this method exists so the proxy still works for
    /// local-disk dev or for buckets that aren't public-read.
    pub async fn read(&self, locator: &str) -> Result<Vec<u8>> {
        match self {
            SuggestionStore::LocalDisk(s) => s.read(locator).await,
            SuggestionStore::R2(r) => r.read(locator).await,
        }
    }

    /// When the store has a public URL (R2 with public-read or a CDN in
    /// front), return it. Local disk returns `None` and the caller should
    /// proxy the bytes through the squintly process.
    pub fn public_url(&self, locator: &str) -> Option<String> {
        match self {
            SuggestionStore::LocalDisk(_) => None,
            SuggestionStore::R2(r) => r.public_url(locator),
        }
    }
}

#[derive(Debug, Clone)]
pub struct StoredObject {
    /// Backend-specific identifier:
    ///   - LocalDisk: absolute filesystem path
    ///   - R2: object key (e.g. `suggestions/de/ad/deadbeef….jpg`)
    pub locator: String,
}

// ---------- LocalDisk ----------

#[derive(Debug, Clone)]
pub struct LocalDiskStore {
    pub base: PathBuf,
}

impl LocalDiskStore {
    pub fn new(base: PathBuf) -> Self {
        Self { base }
    }

    async fn put(&self, prefix: &str, sha: &str, ext: &str, bytes: &[u8]) -> Result<StoredObject> {
        let target = blob_path(&self.base.join(prefix), sha, ext);
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("mkdir {}", parent.display()))?;
        }
        if !target.exists() {
            tokio::fs::write(&target, bytes)
                .await
                .with_context(|| format!("write {}", target.display()))?;
        }
        Ok(StoredObject {
            locator: target.to_string_lossy().to_string(),
        })
    }

    async fn read(&self, locator: &str) -> Result<Vec<u8>> {
        Ok(tokio::fs::read(locator).await?)
    }
}

// ---------- R2 ----------

pub struct R2Store {
    bucket: Bucket,
    credentials: Credentials,
    /// Optional public-read URL prefix (e.g. `https://pub-….r2.dev`). When
    /// set, the `/file` endpoint redirects clients here instead of proxying.
    public_base: Option<String>,
    http: reqwest::Client,
}

impl R2Store {
    pub fn from_env() -> Result<Option<Self>> {
        let endpoint = match std::env::var("SQUINTLY_R2_ENDPOINT").ok() {
            Some(e) if !e.is_empty() => e,
            _ => return Ok(None),
        };
        let bucket_name = std::env::var("SQUINTLY_R2_BUCKET")
            .ok()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                anyhow!("SQUINTLY_R2_BUCKET required when SQUINTLY_R2_ENDPOINT is set")
            })?;
        let access_key = std::env::var("SQUINTLY_R2_ACCESS_KEY_ID")
            .ok()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("SQUINTLY_R2_ACCESS_KEY_ID required"))?;
        let secret_key = std::env::var("SQUINTLY_R2_SECRET_ACCESS_KEY")
            .ok()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("SQUINTLY_R2_SECRET_ACCESS_KEY required"))?;
        let public_base = std::env::var("SQUINTLY_R2_PUBLIC_BASE")
            .ok()
            .filter(|s| !s.is_empty());
        let region = std::env::var("SQUINTLY_R2_REGION").unwrap_or_else(|_| "auto".to_string());

        let endpoint_url = endpoint
            .parse()
            .with_context(|| format!("invalid SQUINTLY_R2_ENDPOINT: {endpoint}"))?;
        let bucket = Bucket::new(endpoint_url, UrlStyle::Path, bucket_name, region)
            .context("constructing R2 bucket")?;
        let credentials = Credentials::new(access_key, secret_key);
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()?;
        Ok(Some(Self {
            bucket,
            credentials,
            public_base,
            http,
        }))
    }

    pub fn bucket_name(&self) -> &str {
        self.bucket.name()
    }

    fn key_for(prefix: &str, sha: &str, ext: &str) -> String {
        let (a, b) = if sha.len() >= 4 {
            (&sha[0..2], &sha[2..4])
        } else {
            ("xx", "xx")
        };
        format!("{prefix}/{a}/{b}/{sha}.{ext}")
    }

    fn public_url(&self, locator: &str) -> Option<String> {
        self.public_base
            .as_deref()
            .map(|base| format!("{}/{}", base.trim_end_matches('/'), locator))
    }

    async fn put(
        &self,
        prefix: &str,
        sha: &str,
        ext: &str,
        bytes: &[u8],
        mime: &str,
    ) -> Result<StoredObject> {
        let key = Self::key_for(prefix, sha, ext);
        let mut action: PutObject = self.bucket.put_object(Some(&self.credentials), &key);
        action
            .headers_mut()
            .insert("content-type", mime.to_string());
        let signed = action.sign(Duration::from_secs(300));
        let resp = self
            .http
            .put(signed)
            .header("content-type", mime)
            .body(bytes.to_vec())
            .send()
            .await
            .context("R2 PUT")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("R2 PUT rejected ({status}): {text}");
        }
        Ok(StoredObject { locator: key })
    }

    async fn read(&self, locator: &str) -> Result<Vec<u8>> {
        // GetObject — sign + GET via reqwest.
        let action = self.bucket.get_object(Some(&self.credentials), locator);
        let signed = action.sign(Duration::from_secs(300));
        let resp = self
            .http
            .get(signed)
            .send()
            .await
            .context("R2 GET")?
            .error_for_status()?;
        Ok(resp.bytes().await?.to_vec())
    }
}

fn blob_path(base: &Path, sha: &str, ext: &str) -> PathBuf {
    let (a, b) = if sha.len() >= 4 {
        (&sha[0..2], &sha[2..4])
    } else {
        ("xx", "xx")
    };
    base.join(a).join(b).join(format!("{sha}.{ext}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn r2_key_layout() {
        let k = R2Store::key_for(
            "suggestions",
            "deadbeef0000000000000000000000000000000000000000000000000000beef",
            "jpg",
        );
        assert_eq!(
            k,
            "suggestions/de/ad/deadbeef0000000000000000000000000000000000000000000000000000beef.jpg"
        );
        // Variants prefix uses the same content-addressed layout under a
        // different top-level directory so the bucket can host both.
        let v = R2Store::key_for(
            "variants",
            "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234",
            "jpg",
        );
        assert_eq!(
            v,
            "variants/ab/cd/abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234.jpg"
        );
    }

    #[tokio::test]
    async fn local_disk_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalDiskStore::new(dir.path().to_path_buf());
        let bytes = b"hello world";
        let obj = store
            .put("test", "aabbccdd11223344", "txt", bytes)
            .await
            .unwrap();
        assert!(obj.locator.contains("aa/bb/aabbccdd11223344.txt"));
        assert!(obj.locator.contains("test"));
        let read = store.read(&obj.locator).await.unwrap();
        assert_eq!(read.as_slice(), bytes);
    }
}
