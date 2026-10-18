use crate::{
    provider::{Candidate, Credentials, Media, Provider, ServiceStopped},
    rom::{cancelled, scan, title},
};
use anyhow::{Context, Result, bail};
use image::{
    DynamicImage, ImageFormat, RgbaImage,
    imageops::{FilterType, overlay},
};
use sha1::{Digest, Sha1};
use std::{
    collections::HashMap,
    fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, mpsc},
    time::Duration,
};

#[derive(Clone)]
pub struct Settings {
    pub root: PathBuf,
    pub output: PathBuf,
    pub extensions: String,
    pub kind: String,
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
            kind: "grids".into(),
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
pub enum Event {
    Log(String),
    Progress(usize, usize),
    Preview(PathBuf),
    Finished,
    Review {
        rom: PathBuf,
        query: String,
        candidates: Vec<Candidate>,
        answer: mpsc::Sender<ReviewAction>,
    },
    ArtworkReview {
        rom: PathBuf,
        game: Candidate,
        media: Vec<Media>,
        previews: Vec<Option<Vec<u8>>>,
        answer: mpsc::Sender<ReviewAction>,
    },
}

pub enum ReviewAction {
    Select(Candidate),
    Search(String),
    LocalImage(PathBuf),
    SelectArtwork(Media),
    Skip,
}

#[derive(Debug)]
pub enum Resolved {
    Game(Candidate, Option<Media>),
    LocalImage(PathBuf),
}

#[derive(Clone, Copy)]
pub struct ResolveOptions<'a> {
    pub auto_first_result: bool,
    pub choose_artwork: bool,
    pub kind: &'a str,
    pub width: u32,
    pub height: u32,
}

pub fn resolve(
    provider: &Provider,
    path: &Path,
    options: ResolveOptions<'_>,
    stop: &AtomicBool,
    emit: &impl Fn(Event),
) -> Result<Option<Resolved>> {
    let mut query = title(path)?;
    loop {
        let candidates = provider.search(&query, stop)?;
        if options.auto_first_result
            && let Some(candidate) = candidates.first()
        {
            return resolve_artwork(provider, path, candidate.clone(), options, stop, emit);
        }
        let (tx, rx) = mpsc::channel();
        emit(Event::Review {
            rom: path.to_path_buf(),
            query: query.clone(),
            candidates: candidates.clone(),
            answer: tx,
        });
        loop {
            cancelled(stop)?;
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(ReviewAction::Select(candidate)) if candidates.contains(&candidate) => {
                    return resolve_artwork(provider, path, candidate, options, stop, emit);
                }
                Ok(ReviewAction::Select(_)) => bail!("Selection is not among the search results"),
                Ok(ReviewAction::Skip) => return Ok(None),
                Ok(ReviewAction::LocalImage(path)) => return Ok(Some(Resolved::LocalImage(path))),
                Ok(ReviewAction::SelectArtwork(_)) => {
                    bail!("Choose a game before choosing artwork")
                }
                Ok(ReviewAction::Search(search)) => {
                    query = search;
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => bail!("Review closed"),
            }
        }
    }
}

fn resolve_artwork(
    provider: &Provider,
    rom: &Path,
    game: Candidate,
    options: ResolveOptions<'_>,
    stop: &AtomicBool,
    emit: &impl Fn(Event),
) -> Result<Option<Resolved>> {
    if !options.choose_artwork || options.kind == "mix3" {
        return Ok(Some(Resolved::Game(game, None)));
    }
    let mut media = match provider.artwork_options(game.id, options.kind, stop) {
        Ok(media) => media,
        Err(error) => {
            return request_local_image(rom, &error.to_string(), stop, emit);
        }
    };
    media.sort_by_key(|m| {
        (
            (m.width > m.height) != (options.width > options.height),
            std::cmp::Reverse(m.score),
        )
    });
    let previews = media
        .iter()
        .map(|item| {
            let bytes = provider.get(&item.url, 4 * 1024 * 1024, stop).ok()?;
            let thumbnail = decode(&bytes).ok()?.thumbnail(150, 200);
            let mut output = Cursor::new(Vec::new());
            thumbnail.write_to(&mut output, ImageFormat::Jpeg).ok()?;
            Some(output.into_inner())
        })
        .collect();
    let (tx, rx) = mpsc::channel();
    emit(Event::ArtworkReview {
        rom: rom.to_path_buf(),
        game: game.clone(),
        media: media.clone(),
        previews,
        answer: tx,
    });
    loop {
        cancelled(stop)?;
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(ReviewAction::SelectArtwork(selected)) if media.contains(&selected) => {
                return Ok(Some(Resolved::Game(game, Some(selected))));
            }
            Ok(ReviewAction::SelectArtwork(_)) => {
                bail!("Artwork selection is not among the results")
            }
            Ok(ReviewAction::Skip) => return Ok(None),
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => bail!("Artwork review closed"),
            _ => bail!("Invalid artwork review action"),
        }
    }
}

fn request_local_image(
    rom: &Path,
    reason: &str,
    stop: &AtomicBool,
    emit: &impl Fn(Event),
) -> Result<Option<Resolved>> {
    let (tx, rx) = mpsc::channel();
    emit(Event::Review {
        rom: rom.to_path_buf(),
        query: format!("Artwork unavailable: {reason}"),
        candidates: Vec::new(),
        answer: tx,
    });
    loop {
        cancelled(stop)?;
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(ReviewAction::LocalImage(path)) => return Ok(Some(Resolved::LocalImage(path))),
            Ok(ReviewAction::Skip) => return Ok(None),
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => bail!("Image selection closed"),
            _ => bail!("Choose a local image or skip this ROM"),
        }
    }
}

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

pub fn fit(image: &DynamicImage, width: u32, height: u32) -> DynamicImage {
    let resized = image.resize(width, height, FilterType::Lanczos3);
    let mut canvas = RgbaImage::new(width, height);
    overlay(
        &mut canvas,
        &resized.to_rgba8(),
        ((width - resized.width()) / 2) as i64,
        ((height - resized.height()) / 2) as i64,
    );
    DynamicImage::ImageRgba8(canvas)
}

fn fetch_image(
    provider: &Provider,
    game: &Candidate,
    kind: &str,
    selected: Option<&Media>,
    settings: &Settings,
    stop: &AtomicBool,
) -> Result<DynamicImage> {
    let (width, height) = match kind {
        "heroes" => (1920, 620),
        "grids" if settings.kind == "mix3" => (600, 900),
        _ => (settings.width, settings.height),
    };
    let media = match selected {
        Some(media) => media.clone(),
        None => provider.artwork(game.id, kind, width, height, stop)?,
    };
    let key = format!("{:x}", Sha1::digest(media.url.as_bytes()));
    let cache = settings.image_directory().join(".rom-art-cache").join(key);
    let bytes = if cache.is_file() {
        fs::read(&cache)?
    } else {
        let bytes = provider.get(&media.url, 32 * 1024 * 1024, stop)?;
        decode(&bytes)?;
        save_atomic(&cache, &bytes, true)?;
        bytes
    };
    decode(&bytes)
}

fn decode(bytes: &[u8]) -> Result<DynamicImage> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    Ok(reader.decode()?)
}

pub fn render(
    provider: &Provider,
    game: &Candidate,
    settings: &Settings,
    selected: Option<&Media>,
    stop: &AtomicBool,
) -> Result<DynamicImage> {
    if settings.kind != "mix3" {
        return Ok(fit(
            &fetch_image(provider, game, &settings.kind, selected, settings, stop)?,
            settings.width,
            settings.height,
        ));
    }
    let mut canvas = RgbaImage::new(settings.width, settings.height);
    // All three components are required: incomplete mixes are reported for review.
    for (kind, x, y, w, h) in [
        ("heroes", 0.10, 0.0, 0.90, 0.80),
        ("grids", 0.0, 0.40, 0.45, 0.60),
        ("logos", 0.50, 0.70, 0.50, 0.30),
    ] {
        let img = fetch_image(provider, game, kind, None, settings, stop)?;
        let img = fit(
            &img,
            (settings.width as f32 * w).max(1.0) as u32,
            (settings.height as f32 * h).max(1.0) as u32,
        );
        overlay(
            &mut canvas,
            &img.to_rgba8(),
            (settings.width as f32 * x) as i64,
            (settings.height as f32 * y) as i64,
        );
    }
    Ok(DynamicImage::ImageRgba8(canvas))
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
                        kind: &settings.kind,
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
                        match render(provider, &game, &settings, media.as_ref(), stop) {
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
    fn fit_preserves_ratio_and_transparency() {
        let source = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            200,
            100,
            image::Rgba([255, 0, 0, 255]),
        ));
        let result = fit(&source, 100, 100).to_rgba8();
        assert_eq!(result.get_pixel(50, 0)[3], 0);
        assert_eq!(result.get_pixel(50, 50)[0], 255);
        assert_eq!(result.get_pixel(50, 99)[3], 0);
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
