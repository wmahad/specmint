//! Schema loading: local file, HTTP(S) with Basic auth + ETag/content-hash cache.

mod cache;
mod fetch;

use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};
use tracing::{debug, info};

use crate::config::{CacheConfig, SourceConfig};
use crate::error::{Error, Result};

pub use cache::{CacheMeta, SchemaCache};

#[derive(Debug, Clone)]
pub struct LoadedSchema {
    pub bytes: Vec<u8>,
    pub content_hash: String,
    pub etag: Option<String>,
    pub from_cache: bool,
}

pub(crate) fn hex_encode(bytes: impl AsRef<[u8]>) -> String {
    hex::encode(bytes.as_ref())
}

impl LoadedSchema {
    pub fn hash_bytes(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hex_encode(hasher.finalize())
    }
}

pub struct SourceLoader {
    cache: SchemaCache,
    use_etag: bool,
    use_content_hash: bool,
}

impl SourceLoader {
    pub fn new(cache_cfg: &CacheConfig) -> Result<Self> {
        Ok(Self {
            cache: SchemaCache::open(&cache_cfg.dir)?,
            use_etag: cache_cfg.use_etag,
            use_content_hash: cache_cfg.use_content_hash,
        })
    }

    pub fn load(&self, source: &SourceConfig, force: bool) -> Result<LoadedSchema> {
        if source.is_url() {
            self.load_url(source.input.trim(), force)
        } else {
            self.load_path(Path::new(source.input.trim()))
        }
    }

    fn load_path(&self, path: &Path) -> Result<LoadedSchema> {
        let bytes = std::fs::read(path)
            .map_err(|e| Error::source(format!("failed to read {}: {e}", path.display())))?;
        let content_hash = LoadedSchema::hash_bytes(&bytes);
        Ok(LoadedSchema {
            bytes,
            content_hash,
            etag: None,
            from_cache: false,
        })
    }

    fn load_url(&self, url: &str, force: bool) -> Result<LoadedSchema> {
        let auth = std::env::var("SPECMINT_BASIC_AUTH")
            .ok()
            .filter(|s| !s.is_empty());
        if auth.is_none() {
            tracing::warn!(
                %url,
                "SPECMINT_BASIC_AUTH unset; fetching without Authorization (401/403 likely)"
            );
        }

        if !force && self.use_content_hash {
            if let Some(cached) = self.cache.read_if_fresh()? {
                if self.use_etag {
                    match fetch::conditional_get(url, auth.as_deref(), cached.meta.etag.as_deref())?
                    {
                        fetch::FetchOutcome::NotModified => {
                            info!("schema unchanged (HTTP 304); using cache");
                            return Ok(LoadedSchema {
                                bytes: cached.bytes,
                                content_hash: cached.meta.content_hash,
                                etag: cached.meta.etag,
                                from_cache: true,
                            });
                        }
                        fetch::FetchOutcome::Body { bytes, etag } => {
                            return self.store_and_return(bytes, etag);
                        }
                    }
                } else {
                    debug!("using content-hash cache without revalidation");
                    return Ok(LoadedSchema {
                        bytes: cached.bytes,
                        content_hash: cached.meta.content_hash,
                        etag: cached.meta.etag,
                        from_cache: true,
                    });
                }
            }
        }

        let outcome = fetch::conditional_get(url, auth.as_deref(), None)?;
        match outcome {
            fetch::FetchOutcome::NotModified => unreachable!("no etag sent"),
            fetch::FetchOutcome::Body { bytes, etag } => self.store_and_return(bytes, etag),
        }
    }

    fn store_and_return(&self, bytes: Vec<u8>, etag: Option<String>) -> Result<LoadedSchema> {
        let content_hash = LoadedSchema::hash_bytes(&bytes);
        self.cache.write(&bytes, &content_hash, etag.as_deref())?;
        Ok(LoadedSchema {
            bytes,
            content_hash,
            etag,
            from_cache: false,
        })
    }
}

pub fn resolve_basic_auth_header(user_pass: &str) -> String {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(user_pass.as_bytes());
    format!("Basic {encoded}")
}

pub fn http_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .user_agent(concat!("specmint/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(Error::from)
}

#[derive(Debug, Clone)]
pub struct CachedBundle {
    pub bytes: Vec<u8>,
    pub meta: CacheMeta,
}

/// Last successful generate fingerprint (parsed-spec hash) for skip-emit.
#[derive(Debug, Clone)]
pub struct GenerateFingerprint {
    pub path: PathBuf,
}

impl GenerateFingerprint {
    pub fn in_cache_dir(cache_dir: &Path) -> Self {
        Self {
            path: cache_dir.join("last-generate.json"),
        }
    }

    pub fn read_spec_hash(&self) -> Result<Option<String>> {
        if !self.path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(&self.path)?;
        let value: serde_json::Value = serde_json::from_str(&raw)?;
        Ok(value
            .get("spec_hash")
            .and_then(|v| v.as_str())
            .map(str::to_owned))
    }

    pub fn write_spec_hash(&self, spec_hash: &str) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = serde_json::json!({
            "spec_hash": spec_hash,
            "written_at": chrono_timestamp(),
        });
        std::fs::write(&self.path, serde_json::to_vec_pretty(&body)?)?;
        Ok(())
    }
}

fn chrono_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    secs.to_string()
}
