use crate::{
    artwork::{self, decode, fit},
    provider::{ArtworkType, Credentials, Provider, ServiceStopped},
    rom::{cancelled, scan, title},
    storage::save_with_backup,
    worker::{Event, ResolveOptions, Resolved, request_local_image, resolve},
};
use anyhow::{Context, Result, bail};
use image::ImageFormat;
use std::{
    collections::HashMap,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

#[derive(Clone)]
pub struct Settings {
    pub root: PathBuf,
    pub output: PathBuf,
    pub extensions: String,
    pub kind: ArtworkType,
    pub width: u32,
    pub height: u32,
    pub overwrite: bool,
    pub backup_existing: bool,
    pub auto_first_result: bool,
    pub choose_artwork: bool,
    pub only_missing: bool,
    pub gamelist: bool,
    pub onion_images: bool,
    pub gamelist_format: crate::gamelist::Format,
    pub credentials: Credentials,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            root: PathBuf::new(),
            output: PathBuf::new(),
            extensions: "nes,sfc,smc,gb,gbc,gba,md,gen,sms,gg,pce,nds,z64,n64,v64,iso,chd,pbp,zip"
                .into(),
            kind: ArtworkType::Grid,
            width: 250,
            height: 360,
            overwrite: false,
            backup_existing: true,
            auto_first_result: true,
            choose_artwork: false,
            only_missing: false,
            gamelist: true,
            onion_images: false,
            gamelist_format: crate::gamelist::Format::EmulationStation,
            credentials: Credentials::default(),
        }
    }
}
impl Settings {
    pub fn image_directory(&self) -> PathBuf {
        if self.onion_images {
            self.root.join("Imgs")
        } else {
            self.output.clone()
        }
    }
    pub fn onion_preset(&mut self, width: u32, height: u32) {
        self.onion_images = true;
        self.width = width;
        self.height = height;
    }
}
pub fn run(settings: Settings, stop: &AtomicBool, emit: impl Fn(Event)) -> Result<()> {
    let provider = Provider::new(settings.credentials.clone())?;
    run_with_provider(settings, &provider, stop, emit)
}

pub fn image_path(settings: &Settings, rom: &Path) -> Result<PathBuf> {
    let mut path = settings
        .image_directory()
        .join(rom.strip_prefix(&settings.root)?);
    if settings.onion_images {
        path.set_extension("png");
    } else {
        let mut filename = path.file_name().context("Invalid filename")?.to_os_string();
        filename.push(".png");
        path.set_file_name(filename);
    }
    Ok(path)
}

fn validate_image_paths(settings: &Settings, paths: &[PathBuf]) -> Result<()> {
    let mut outputs = HashMap::new();
    for rom in paths {
        let path = image_path(settings, rom)?;
        let key = if settings.onion_images {
            path.to_string_lossy().to_lowercase()
        } else {
            path.to_string_lossy().into_owned()
        };
        if let Some(previous) = outputs.insert(key, rom) {
            bail!(
                "Artwork filename collision: {} and {}. Use separate system folders or disable OnionOS naming",
                previous.display(),
                rom.display()
            );
        }
    }
    Ok(())
}

fn write_gamelist(
    settings: &Settings,
    paths: &[PathBuf],
    names: &HashMap<PathBuf, String>,
    emit: &impl Fn(Event),
) -> Result<()> {
    let mut entries = Vec::new();
    for path in paths {
        let image = image_path(settings, path)?;
        let image = if image.is_file() && image::image_dimensions(&image).is_ok() {
            Some(image)
        } else {
            None
        };
        entries.push(crate::gamelist::Entry {
            rom: path.clone(),
            name: names.get(path).cloned().unwrap_or(title(path)?),
            image,
        });
    }
    for backup in crate::gamelist::export(
        &settings.root,
        &entries,
        settings.backup_existing,
        settings.gamelist_format,
    )? {
        emit(Event::Log(format!(
            "Game list backup: {}",
            backup.display()
        )));
    }
    emit(Event::Log(format!(
        "Game list updated: {} ({} ROMs in the folder; existing entries retained)",
        settings.gamelist_format.label(),
        paths.len()
    )));
    Ok(())
}

pub fn generate_local_gamelist(
    settings: Settings,
    stop: &AtomicBool,
    emit: impl Fn(Event),
) -> Result<()> {
    if !settings.onion_images && settings.output.as_os_str().is_empty() {
        bail!("Select an artwork folder");
    }
    let paths = scan(&settings.root, &settings.extensions)?;
    validate_image_paths(&settings, &paths)?;
    cancelled(stop)?;
    write_gamelist(&settings, &paths, &HashMap::new(), &emit)
}

pub fn run_with_provider(
    settings: Settings,
    provider: &Provider,
    stop: &AtomicBool,
    emit: impl Fn(Event),
) -> Result<()> {
    if !settings.onion_images && settings.output.as_os_str().is_empty() {
        bail!("Select an output folder");
    }
    if settings.credentials.api_key.trim().is_empty() {
        bail!("Enter your personal SteamGridDB API key on the Connection page");
    }
    if !(1..=4096).contains(&settings.width) || !(1..=4096).contains(&settings.height) {
        bail!("Allowed dimensions: 1 to 4096");
    }
    let all_paths = scan(&settings.root, &settings.extensions)?;
    let paths = if settings.only_missing {
        all_paths
            .iter()
            .filter(|&rom| image_path(&settings, rom).is_ok_and(|p| !p.is_file()))
            .cloned()
            .collect::<Vec<_>>()
    } else {
        all_paths.clone()
    };
    validate_image_paths(&settings, &paths)?;
    let total = paths.len();
    emit(Event::Log(format!("Found {total} ROMs")));
    let mut saved = 0;
    let mut skipped = 0;
    let mut failed = 0;
    let mut names = HashMap::new();
    for (index, path) in paths.iter().enumerate() {
        cancelled(stop)?;
        // Preserve the relative directories AND extension to prevent same-stem collisions.
        let relative = path.strip_prefix(&settings.root)?;
        let output = image_path(&settings, path)?;
        if output.exists() && !settings.overwrite {
            skipped += 1;
            emit(Event::Log(format!("Kept: {}", relative.display())));
        } else {
            emit(Event::Log(format!("Matching: {}", relative.display())));
            let result = (|| -> Result<Option<String>> {
                let Some(resolved) = resolve(
                    provider,
                    path,
                    ResolveOptions {
                        auto_first_result: settings.auto_first_result,
                        choose_artwork: settings.choose_artwork,
                        kind: settings.kind,
                        width: settings.width,
                        height: settings.height,
                    },
                    stop,
                    &emit,
                )?
                else {
                    return Ok(None);
                };
                let (image, name) = match resolved {
                    Resolved::Game(game, media) => {
                        match artwork::render(
                            provider,
                            &game,
                            &artwork::Options {
                                kind: settings.kind,
                                width: settings.width,
                                height: settings.height,
                                cache_directory: &settings.image_directory().join(".rom-art-cache"),
                            },
                            media.as_ref(),
                            stop,
                        ) {
                            Ok(image) => (image, game.name),
                            Err(error) => {
                                let Some(Resolved::LocalImage(local)) =
                                    request_local_image(path, &error.to_string(), stop, &emit)?
                                else {
                                    return Ok(None);
                                };
                                (
                                    fit(
                                        &decode(&fs::read(local)?)?,
                                        settings.width,
                                        settings.height,
                                    ),
                                    title(path)?,
                                )
                            }
                        }
                    }
                    Resolved::LocalImage(local) => (
                        fit(&decode(&fs::read(local)?)?, settings.width, settings.height),
                        title(path)?,
                    ),
                };
                let mut png = Cursor::new(Vec::new());
                image.write_to(&mut png, ImageFormat::Png)?;
                cancelled(stop)?;
                if let Some(backup) = save_with_backup(
                    &output,
                    png.get_ref(),
                    settings.overwrite,
                    settings.backup_existing,
                )? {
                    emit(Event::Log(format!("Artwork backup: {}", backup.display())));
                }
                Ok(Some(name))
            })();
            match result {
                Ok(Some(name)) => {
                    names.insert(path.clone(), name.clone());
                    saved += 1;
                    emit(Event::Log(format!("Saved: {name} → {}", output.display())));
                    emit(Event::Preview(output));
                }
                Ok(None) => {
                    skipped += 1;
                    emit(Event::Log(format!("Skipped: {}", relative.display())));
                }
                Err(e) => {
                    cancelled(stop)?;
                    if e.downcast_ref::<ServiceStopped>().is_some() {
                        return Err(e);
                    }
                    failed += 1;
                    emit(Event::Log(format!("Review {}: {e}", relative.display())));
                }
            }
        }
        emit(Event::Progress(index + 1, total));
    }
    if settings.gamelist {
        write_gamelist(&settings, &all_paths, &names, &emit)?;
    }
    emit(Event::Log(format!(
        "Finished: {saved} images saved, {skipped} kept or skipped, {failed} need review"
    )));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::save_atomic;
    #[test]
    fn onion_presets_set_sizes_folder_and_matching_filenames() {
        let root = PathBuf::from("/roms/GBA");
        let mut settings = Settings {
            root: root.clone(),
            output: "/other".into(),
            ..Default::default()
        };
        assert_eq!(
            image_path(&settings, &root.join("Game.gba")).unwrap(),
            PathBuf::from("/other/Game.gba.png")
        );
        for (width, height) in [(250, 360), (250, 250), (250, 180)] {
            settings.onion_preset(width, height);
            assert_eq!((settings.width, settings.height), (width, height));
            assert_eq!(
                image_path(&settings, &root.join("sub/Game.gba")).unwrap(),
                root.join("Imgs/sub/Game.png")
            );
        }
        assert!(
            validate_image_paths(&settings, &[root.join("Game.zip"), root.join("Game.gba")])
                .is_err()
        );
        assert!(
            validate_image_paths(&settings, &[root.join("Game.zip"), root.join("GAME.gba")])
                .is_err()
        );
    }
    #[test]
    fn backups_are_unique_and_optional() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cover.png");
        fs::write(&path, b"v1").unwrap();
        let first = save_with_backup(&path, b"v2", true, true).unwrap().unwrap();
        let second = save_with_backup(&path, b"v3", true, true).unwrap().unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read(first).unwrap(), b"v1");
        assert_eq!(fs::read(second).unwrap(), b"v2");
        assert!(
            save_with_backup(&path, b"v4", true, false)
                .unwrap()
                .is_none()
        );
        assert_eq!(fs::read(&path).unwrap(), b"v4");
        assert!(save_with_backup(&path, b"v5", false, true).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"v4");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 3);
    }
    #[test]
    fn existing_files_remain_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a");
        save_atomic(&path, b"original", false).unwrap();
        assert!(save_atomic(&path, b"replacement", false).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"original");
        save_atomic(&path, b"replacement", true).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"replacement");
    }
}
