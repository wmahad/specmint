use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::source::CachedBundle;

const SCHEMA_FILE: &str = "schema.yml";
const META_FILE: &str = "meta.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheMeta {
    pub content_hash: String,
    pub etag: Option<String>,
    pub fetched_at: String,
}

#[derive(Debug)]
pub struct SchemaCache {
    dir: PathBuf,
}

impl SchemaCache {
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        Ok(Self {
            dir: dir.to_path_buf(),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn read_if_fresh(&self) -> Result<Option<CachedBundle>> {
        let meta_path = self.dir.join(META_FILE);
        let schema_path = self.dir.join(SCHEMA_FILE);
        if !meta_path.exists() || !schema_path.exists() {
            return Ok(None);
        }
        let meta: CacheMeta = serde_json::from_str(&std::fs::read_to_string(meta_path)?)?;
        let bytes = std::fs::read(schema_path)?;
        let actual = crate::source::LoadedSchema::hash_bytes(&bytes);
        if actual != meta.content_hash {
            return Err(Error::source(
                "cache corrupted: schema hash does not match meta.json",
            ));
        }
        Ok(Some(CachedBundle { bytes, meta }))
    }

    pub fn write(&self, bytes: &[u8], content_hash: &str, etag: Option<&str>) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.dir.join(SCHEMA_FILE), bytes)?;
        let meta = CacheMeta {
            content_hash: content_hash.to_string(),
            etag: etag.map(str::to_owned),
            fetched_at: super::chrono_timestamp(),
        };
        std::fs::write(self.dir.join(META_FILE), serde_json::to_vec_pretty(&meta)?)?;
        Ok(())
    }
}
