//! Write generated files; optionally clean the output directory.

use std::path::{Path, PathBuf};

use crate::emit::OutputFile;
use crate::error::{Error, Result};

const PRESERVE_NAMES: &[&str] = &[".gitkeep"];

pub fn write_outputs(output_dir: &Path, clean: bool, files: &[OutputFile]) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(output_dir)?;

    if clean {
        clean_dir(output_dir)?;
    }

    let mut written = Vec::new();
    for file in files {
        let path = output_dir.join(&file.relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, &file.contents)?;
        written.push(path);
    }
    Ok(written)
}

fn clean_dir(dir: &Path) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if PRESERVE_NAMES.contains(&name.as_ref()) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            std::fs::remove_dir_all(&path).map_err(|e| {
                Error::Other(anyhow::anyhow!("failed to remove {}: {e}", path.display()))
            })?;
        } else {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}

/// Returns true when all configured output files already exist (used with IR-hash skip).
pub fn outputs_exist(output_dir: &Path, relative_files: &[PathBuf]) -> bool {
    relative_files
        .iter()
        .all(|rel| output_dir.join(rel).is_file())
}
