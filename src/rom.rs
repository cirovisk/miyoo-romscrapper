use anyhow::{Context, Result, bail};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

pub fn cancelled(stop: &AtomicBool) -> Result<()> {
    if stop.load(Ordering::Relaxed) {
        bail!("Cancelled");
    }
    Ok(())
}

/// Remove only recognized trailing dump tags, preserving subtitles and hack names.
pub fn title(path: &Path) -> Result<String> {
    let mut name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .context("Invalid ROM filename")?
        .trim()
        .to_string();
    loop {
        let open = if name.ends_with(')') {
            '('
        } else if name.ends_with(']') {
            '['
        } else {
            break;
        };
        let Some(start) = name.rfind(open) else {
            break;
        };
        let tag = name[start + 1..name.len() - 1].trim().to_lowercase();
        let recognized = tag == "!"
            || tag.starts_with("rev ")
            || tag.starts_with("disc ")
            || tag.starts_with("disk ")
            || tag.split([',', '+']).all(|s| {
                matches!(
                    s.trim(),
                    "usa"
                        | "europe"
                        | "japan"
                        | "world"
                        | "brazil"
                        | "korea"
                        | "france"
                        | "germany"
                        | "spain"
                        | "italy"
                        | "australia"
                        | "canada"
                        | "en"
                        | "ja"
                        | "fr"
                        | "de"
                        | "es"
                        | "it"
                        | "pt"
                        | "nl"
                        | "ko"
                        | "zh"
                )
            });
        if !recognized {
            break;
        }
        name.truncate(start);
        name = name.trim().to_string();
    }
    if name.is_empty() {
        bail!("ROM filename has no title");
    }
    Ok(name)
}

pub fn scan(root: &Path, extensions: &str) -> Result<Vec<PathBuf>> {
    if !root.is_dir() {
        bail!("Select an existing ROM folder");
    }
    let extensions: Vec<_> = extensions
        .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .map(|s| s.trim().trim_start_matches('.').to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if extensions.is_empty() {
        bail!("Enter at least one file extension");
    }
    let mut paths = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'))
    {
        let entry = entry?;
        if entry.file_type().is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|e| extensions.contains(&e.to_string_lossy().to_lowercase()))
        {
            paths.push(entry.path().to_path_buf());
        }
    }
    paths.sort();
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn cleans_dump_tags_without_erasing_hacks_or_subtitles() {
        assert_eq!(
            title(Path::new("Sonic (USA, Europe) (Rev 1) [!].zip")).unwrap(),
            "Sonic"
        );
        assert_eq!(
            title(Path::new("Game (The Adventure) (USA).nes")).unwrap(),
            "Game (The Adventure)"
        );
        assert_eq!(
            title(Path::new("Mario (Hack) (Japan).nes")).unwrap(),
            "Mario (Hack)"
        );
        assert_eq!(title(Path::new("A & B.zip")).unwrap(), "A & B");
        assert!(title(Path::new("(USA).zip")).is_err());
    }

    #[test]
    fn scans_rvz_and_accepts_common_extension_separators() {
        let root = tempfile::tempdir().unwrap();
        let rvz = root.path().join("Game.rvz");
        let zip = root.path().join("Other.ZIP");
        let ignored = root.path().join("readme.txt");
        fs::write(&rvz, b"rom").unwrap();
        fs::write(&zip, b"rom").unwrap();
        fs::write(&ignored, b"text").unwrap();

        assert_eq!(scan(root.path(), ".rvz; ZIP").unwrap(), vec![rvz, zip]);
    }
}
