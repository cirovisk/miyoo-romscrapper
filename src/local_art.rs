use crate::{
    engine::{Settings, image_path},
    rom::{scan, title},
};
use anyhow::{Context, Result, bail};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct ImportItem {
    pub rom: PathBuf,
    pub source: PathBuf,
    pub output: PathBuf,
    pub width: u32,
    pub height: u32,
    pub overwrite: bool,
    pub backup_existing: bool,
}

#[derive(Clone)]
pub struct AmbiguousImport {
    pub rom: PathBuf,
    pub output: PathBuf,
    pub candidates: Vec<PathBuf>,
    pub selected: Option<PathBuf>,
    pub width: u32,
    pub height: u32,
    pub overwrite: bool,
    pub backup_existing: bool,
}

#[derive(Clone, Default)]
pub struct ImportPreview {
    pub items: Vec<ImportItem>,
    pub ambiguous: Vec<AmbiguousImport>,
    pub unmatched: usize,
    pub kept: usize,
}

pub fn preview(settings: &[Settings], source_root: &Path) -> Result<ImportPreview> {
    if !source_root.is_dir() {
        bail!("Select an existing local artwork folder");
    }
    let sources = index_images(source_root)?;
    let mut result = ImportPreview::default();
    for system in settings {
        let paths = scan(&system.root, &system.extensions)?;
        let mut outputs = HashMap::new();
        for rom in paths {
            let output = image_path(system, &rom)?;
            let output_key = if system.onion_images {
                output.to_string_lossy().to_ascii_lowercase()
            } else {
                output.to_string_lossy().into_owned()
            };
            if let Some(previous) = outputs.insert(output_key, rom.clone()) {
                bail!(
                    "Artwork output collision: {} and {}",
                    previous.display(),
                    rom.display()
                );
            }
            let keys = match_keys(&rom)?;
            let candidates = find_candidates(&sources, &keys, source_root, &system.system_id);
            match candidates.as_slice() {
                [_source] if output.is_file() && !system.overwrite => result.kept += 1,
                [source] => result.items.push(ImportItem {
                    rom: rom.clone(),
                    source: source.clone(),
                    output,
                    width: system.width,
                    height: system.height,
                    overwrite: system.overwrite,
                    backup_existing: system.backup_existing,
                }),
                [] => result.unmatched += 1,
                many => result.ambiguous.push(AmbiguousImport {
                    rom,
                    output,
                    candidates: many.to_vec(),
                    selected: None,
                    width: system.width,
                    height: system.height,
                    overwrite: system.overwrite,
                    backup_existing: system.backup_existing,
                }),
            }
        }
    }
    Ok(result)
}

fn index_images(root: &Path) -> Result<HashMap<String, Vec<PathBuf>>> {
    let mut index = HashMap::<String, Vec<PathBuf>>::new();
    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0 || !entry.file_name().to_string_lossy().starts_with('.')
        })
    {
        let entry = entry?;
        if !entry.file_type().is_file() || !is_supported_image(entry.path()) {
            continue;
        }
        for key in match_keys(entry.path())? {
            index
                .entry(key)
                .or_default()
                .push(entry.path().to_path_buf());
        }
    }
    for paths in index.values_mut() {
        paths.sort();
        paths.dedup();
    }
    Ok(index)
}

fn is_supported_image(path: &Path) -> bool {
    path.extension().is_some_and(|extension| {
        ["png", "jpg", "jpeg", "webp"]
            .iter()
            .any(|candidate| extension.eq_ignore_ascii_case(candidate))
    })
}

fn match_keys(path: &Path) -> Result<Vec<String>> {
    let mut keys = vec![normalize(&title(path)?)];
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .context("Invalid artwork filename")?;
    if let Some(inner_extension) = Path::new(stem).extension().and_then(|ext| ext.to_str())
        && is_rom_extension(inner_extension)
    {
        let key = normalize(
            Path::new(stem)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or(stem),
        );
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    Ok(keys)
}

fn is_rom_extension(extension: &str) -> bool {
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "nes"
            | "unf"
            | "unif"
            | "fds"
            | "sfc"
            | "smc"
            | "gb"
            | "gbc"
            | "gba"
            | "nds"
            | "n64"
            | "z64"
            | "v64"
            | "3ds"
            | "cia"
            | "md"
            | "gen"
            | "smd"
            | "32x"
            | "sms"
            | "gg"
            | "pce"
            | "sgx"
            | "a26"
            | "a52"
            | "a78"
            | "lnx"
            | "j64"
            | "jag"
            | "ngp"
            | "ngc"
            | "ws"
            | "wsc"
            | "d64"
            | "d71"
            | "d81"
            | "t64"
            | "tap"
            | "prg"
            | "adf"
            | "iso"
            | "cso"
            | "chd"
            | "cue"
            | "bin"
            | "pbp"
            | "m3u"
            | "cdi"
            | "gdi"
            | "rvz"
            | "wbfs"
            | "wud"
            | "wux"
            | "xci"
            | "nsp"
            | "zip"
            | "7z"
    )
}

fn normalize(value: &str) -> String {
    value.trim().to_lowercase()
}

fn find_candidates(
    index: &HashMap<String, Vec<PathBuf>>,
    keys: &[String],
    source_root: &Path,
    system_id: &str,
) -> Vec<PathBuf> {
    let mut all = keys
        .iter()
        .filter_map(|key| index.get(key))
        .flatten()
        .cloned()
        .collect::<Vec<_>>();
    all.sort();
    all.dedup();
    if !system_id.is_empty() {
        let preferred = all
            .iter()
            .filter(|path| has_system_component(path, source_root, system_id))
            .cloned()
            .collect::<Vec<_>>();
        if !preferred.is_empty() {
            return preferred;
        }
    }
    all
}

fn has_system_component(path: &Path, root: &Path, system_id: &str) -> bool {
    let mapping = crate::systems::mapping(system_id);
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .any(|component| {
            component.eq_ignore_ascii_case(system_id)
                || mapping.is_some_and(|mapping| {
                    mapping
                        .platforms
                        .split_whitespace()
                        .any(|platform| component.eq_ignore_ascii_case(platform))
                })
        })
}

impl ImportPreview {
    pub fn selected_items(&self) -> Vec<ImportItem> {
        let mut items = self.items.clone();
        for ambiguous in &self.ambiguous {
            if let Some(source) = &ambiguous.selected {
                items.push(ImportItem {
                    rom: ambiguous.rom.clone(),
                    source: source.clone(),
                    output: ambiguous.output.clone(),
                    width: ambiguous.width,
                    height: ambiguous.height,
                    overwrite: ambiguous.overwrite,
                    backup_existing: ambiguous.backup_existing,
                });
            }
        }
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::systems;
    use std::fs;

    fn settings(root: &Path, output: &Path) -> Settings {
        Settings {
            root: root.to_path_buf(),
            output: output.to_path_buf(),
            extensions: "zip".into(),
            system_id: "GB".into(),
            ..Settings::default()
        }
    }

    #[test]
    fn local_images_match_by_title_and_prefer_the_system_platform_folder() {
        let root = tempfile::tempdir().unwrap();
        let rom_dir = root.path().join("roms/GB");
        let source_dir = root.path().join("import");
        let output_dir = root.path().join("artwork");
        fs::create_dir_all(&rom_dir).unwrap();
        fs::create_dir_all(source_dir.join("misc")).unwrap();
        fs::create_dir_all(source_dir.join("GB/screenshot")).unwrap();
        fs::write(rom_dir.join("Game.zip"), b"rom").unwrap();
        fs::write(source_dir.join("misc/Game.png"), b"other system").unwrap();
        fs::write(
            source_dir.join("GB/screenshot/Game.zip.png"),
            b"selected system",
        )
        .unwrap();

        let preview = preview(&[settings(&rom_dir, &output_dir)], &source_dir).unwrap();
        assert_eq!(preview.items.len(), 1);
        assert_eq!(
            preview.items[0].source,
            source_dir.join("GB/screenshot/Game.zip.png")
        );
        assert_eq!(preview.ambiguous.len(), 0);
        assert_eq!(systems::mapping("GB").unwrap().platforms, "gb gbc");
    }

    #[test]
    fn ambiguous_local_images_require_an_explicit_choice() {
        let root = tempfile::tempdir().unwrap();
        let rom_dir = root.path().join("roms/GB");
        let source_dir = root.path().join("import");
        fs::create_dir_all(&rom_dir).unwrap();
        fs::create_dir_all(source_dir.join("GB/screenshot")).unwrap();
        fs::create_dir_all(source_dir.join("GB/wheel")).unwrap();
        fs::write(rom_dir.join("Game.zip"), b"rom").unwrap();
        fs::write(source_dir.join("GB/screenshot/Game.png"), b"first").unwrap();
        fs::write(source_dir.join("GB/wheel/Game.png"), b"second").unwrap();

        let mut preview =
            preview(&[settings(&rom_dir, &root.path().join("art"))], &source_dir).unwrap();
        assert_eq!(preview.ambiguous.len(), 1);
        assert!(preview.selected_items().is_empty());
        preview.ambiguous[0].selected = Some(source_dir.join("GB/wheel/Game.png"));
        let selected = preview.selected_items();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].source, source_dir.join("GB/wheel/Game.png"));
        assert_eq!((selected[0].width, selected[0].height), (250, 360));
    }
}
