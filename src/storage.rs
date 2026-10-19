use anyhow::{Context, Result};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

/// Write a complete file in the destination directory before making it visible.
pub fn save_atomic(path: &Path, bytes: &[u8], overwrite: bool) -> Result<()> {
    let parent = path
        .parent()
        .context("Destination has no parent directory")?;
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    if overwrite {
        temp.persist(path).map_err(|e| e.error)?;
    } else {
        temp.persist_noclobber(path).map_err(|e| e.error)?;
    }
    Ok(())
}

pub fn save_with_backup(
    path: &Path,
    bytes: &[u8],
    overwrite: bool,
    backup: bool,
) -> Result<Option<PathBuf>> {
    let backup = if overwrite && backup && path.exists() {
        let original = fs::read(path)?;
        let parent = path
            .parent()
            .context("Destination has no parent directory")?;
        let prefix = format!(
            "{}.backup-",
            path.file_name()
                .context("Invalid filename")?
                .to_string_lossy()
        );
        let suffix = path
            .extension()
            .map(|ext| format!(".{}", ext.to_string_lossy()))
            .unwrap_or_default();
        let mut file = tempfile::Builder::new()
            .prefix(&prefix)
            .suffix(&suffix)
            .tempfile_in(parent)?;
        file.write_all(&original)?;
        file.as_file().sync_all()?;
        let (_, backup) = file.keep().map_err(|e| e.error)?;
        Some(backup)
    } else {
        None
    };
    save_atomic(path, bytes, overwrite)?;
    Ok(backup)
}

pub fn app_data_dir() -> Result<PathBuf> {
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    }
    .context("Could not locate the application settings directory")?;
    Ok(base.join("Miyoo ROMScrapper"))
}
