use crate::{
    artwork::{self, decode, fit},
    provider::{ArtworkType, Candidate, Credentials, Provider, ServiceStopped},
    rom::{cancelled, scan, title},
    storage::save_with_backup,
    worker::{
        Event, ResolveOptions, Resolved, initial_search_query, request_local_image,
        resolve_with_initial_search,
    },
};
use anyhow::{Context, Result, bail};
use image::ImageFormat;
use std::{
    cell::Cell,
    collections::HashMap,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

pub const DEFAULT_CONCURRENCY: usize = 3;
pub const MAX_CONCURRENCY: usize = 8;

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
    pub turbo_mode: bool,
    pub concurrency: usize,
    pub arcade_mode: bool,
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
            extensions: "nes,unf,unif,fds,sfc,smc,gb,gbc,gba,nds,n64,z64,v64,3ds,cia,md,gen,smd,32x,sms,gg,pce,sgx,a26,a52,a78,lnx,j64,jag,ngp,ngc,ws,wsc,d64,d71,d81,t64,tap,prg,adf,iso,cso,chd,cue,bin,pbp,m3u,cdi,gdi,rvz,wbfs,wud,wux,xci,nsp,zip,7z"
                .into(),
            kind: ArtworkType::Grid,
            width: 250,
            height: 360,
            overwrite: false,
            backup_existing: true,
            auto_first_result: true,
            turbo_mode: false,
            concurrency: DEFAULT_CONCURRENCY,
            arcade_mode: false,
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
    let cache_directory = settings.image_directory().join(".rom-art-cache");
    let mut cleanup = CacheCleanup(cache_directory.clone());
    let result = run_with_provider_inner(settings, provider, stop, &emit, &cache_directory);
    if let Err(error) = cleanup.clear() {
        emit(Event::Log(format!(
            "Could not remove temporary artwork cache: {error}"
        )));
        if result.is_ok() {
            return Err(error.into());
        }
    }
    result
}

struct CacheCleanup(PathBuf);

impl CacheCleanup {
    fn clear(&mut self) -> std::io::Result<()> {
        match std::fs::remove_dir_all(&self.0) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

impl Drop for CacheCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

type SearchResult = Option<std::result::Result<Vec<Candidate>, String>>;

struct SearchPipeline<'scope, 'env: 'scope> {
    scope: &'scope std::thread::Scope<'scope, 'env>,
    provider: &'env Provider,
    paths: &'env [PathBuf],
    settings: &'env Settings,
    stop: &'env AtomicBool,
    sender: mpsc::Sender<(usize, SearchResult)>,
    receiver: mpsc::Receiver<(usize, SearchResult)>,
    completed: HashMap<usize, SearchResult>,
    next_to_launch: usize,
}

struct DownloadJob {
    path: PathBuf,
    output: PathBuf,
    relative: String,
    resolved: Resolved,
}

struct DownloadResult {
    job: DownloadJob,
    result: Result<(String, Option<PathBuf>)>,
}

struct DownloadStats {
    names: HashMap<PathBuf, String>,
    saved: usize,
    skipped: usize,
    no_match: usize,
    failed: usize,
    completed: usize,
    total: usize,
}

struct DownloadPipeline<'scope, 'env: 'scope> {
    scope: &'scope std::thread::Scope<'scope, 'env>,
    provider: &'env Provider,
    settings: &'env Settings,
    stop: &'env AtomicBool,
    cache_directory: &'env Path,
    sender: mpsc::Sender<DownloadResult>,
    receiver: mpsc::Receiver<DownloadResult>,
    active: usize,
    concurrency: usize,
}

impl<'scope, 'env: 'scope> DownloadPipeline<'scope, 'env> {
    fn new(
        scope: &'scope std::thread::Scope<'scope, 'env>,
        provider: &'env Provider,
        settings: &'env Settings,
        stop: &'env AtomicBool,
        cache_directory: &'env Path,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            scope,
            provider,
            settings,
            stop,
            cache_directory,
            sender,
            receiver,
            active: 0,
            concurrency: settings.concurrency.clamp(1, MAX_CONCURRENCY),
        }
    }

    fn is_full(&self) -> bool {
        self.active >= self.concurrency
    }

    fn spawn(&mut self, job: DownloadJob) {
        let provider = self.provider;
        let settings = self.settings;
        let stop = self.stop;
        let cache_directory = self.cache_directory;
        let sender = self.sender.clone();
        self.active += 1;
        self.scope.spawn(move || {
            let result = process_download(provider, settings, cache_directory, stop, &job);
            let _ = sender.send(DownloadResult { job, result });
        });
    }

    fn take(&mut self) -> Result<DownloadResult> {
        let result = self
            .receiver
            .recv()
            .context("Artwork download worker stopped")?;
        self.active -= 1;
        Ok(result)
    }
}

fn process_download(
    provider: &Provider,
    settings: &Settings,
    cache_directory: &Path,
    stop: &AtomicBool,
    job: &DownloadJob,
) -> Result<(String, Option<PathBuf>)> {
    let (image, name) = match &job.resolved {
        Resolved::Game(game, media) => (
            artwork::render(
                provider,
                game,
                &artwork::Options {
                    kind: settings.kind,
                    width: settings.width,
                    height: settings.height,
                    cache_directory,
                },
                media.as_ref(),
                stop,
            )?,
            game.name.clone(),
        ),
        Resolved::LocalImage(local) => (
            fit(&decode(&fs::read(local)?)?, settings.width, settings.height),
            title(&job.path)?,
        ),
        Resolved::NoMatch => bail!("Cannot download artwork for a ROM without a game match"),
    };
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, ImageFormat::Png)?;
    cancelled(stop)?;
    let backup = save_with_backup(
        &job.output,
        png.get_ref(),
        settings.overwrite,
        settings.backup_existing,
    )?;
    Ok((name, backup))
}

fn finish_download(
    mut result: DownloadResult,
    stop: &AtomicBool,
    emit: &impl Fn(Event),
    stats: &mut DownloadStats,
) -> Result<Option<DownloadJob>> {
    match result.result {
        Ok((name, backup)) => {
            if let Some(backup) = backup {
                emit(Event::Log(format!("Artwork backup: {}", backup.display())));
            }
            stats.names.insert(result.job.path.clone(), name.clone());
            stats.saved += 1;
            emit(Event::Log(format!(
                "Saved: {name} → {}",
                result.job.output.display()
            )));
            emit(Event::Preview(result.job.output));
            stats.completed += 1;
            emit(Event::Progress(stats.completed, stats.total));
            Ok(None)
        }
        Err(error) => {
            cancelled(stop)?;
            if error.downcast_ref::<ServiceStopped>().is_some() {
                return Err(error);
            }
            if matches!(&result.job.resolved, Resolved::Game(_, _)) {
                let Some(Resolved::LocalImage(local)) =
                    request_local_image(&result.job.path, &error.to_string(), stop, emit)?
                else {
                    stats.skipped += 1;
                    stats.completed += 1;
                    emit(Event::Log(format!("Skipped: {}", result.job.relative)));
                    emit(Event::Progress(stats.completed, stats.total));
                    return Ok(None);
                };
                result.job.resolved = Resolved::LocalImage(local);
                Ok(Some(result.job))
            } else {
                stats.failed += 1;
                stats.completed += 1;
                emit(Event::Log(format!(
                    "Review {}: {error}",
                    result.job.relative
                )));
                emit(Event::Progress(stats.completed, stats.total));
                Ok(None)
            }
        }
    }
}

impl<'scope, 'env: 'scope> SearchPipeline<'scope, 'env> {
    fn new(
        scope: &'scope std::thread::Scope<'scope, 'env>,
        provider: &'env Provider,
        paths: &'env [PathBuf],
        settings: &'env Settings,
        stop: &'env AtomicBool,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        let mut pipeline = Self {
            scope,
            provider,
            paths,
            settings,
            stop,
            sender,
            receiver,
            completed: HashMap::new(),
            next_to_launch: 0,
        };
        for _ in 0..settings
            .concurrency
            .clamp(1, MAX_CONCURRENCY)
            .min(paths.len())
        {
            pipeline.launch_next();
        }
        pipeline
    }

    fn launch_next(&mut self) {
        if self.next_to_launch >= self.paths.len() || self.stop.load(Ordering::Relaxed) {
            return;
        }
        let index = self.next_to_launch;
        self.next_to_launch += 1;
        let path = &self.paths[index];
        let settings = self.settings;
        let provider = self.provider;
        let stop = self.stop;
        let sender = self.sender.clone();
        self.scope.spawn(move || {
            let result =
                if !settings.overwrite && image_path(settings, path).is_ok_and(|p| p.exists()) {
                    None
                } else {
                    Some(
                        initial_search_query(path, Some(&settings.root), settings.arcade_mode)
                            .and_then(|query| provider.search(&query, stop))
                            .map_err(|error| error.to_string()),
                    )
                };
            let _ = sender.send((index, result));
        });
    }

    fn take(&mut self, index: usize) -> Result<SearchResult> {
        loop {
            cancelled(self.stop)?;
            if let Some(result) = self.completed.remove(&index) {
                return Ok(result);
            }
            match self.receiver.recv_timeout(Duration::from_millis(100)) {
                Ok((completed_index, result)) => {
                    self.completed.insert(completed_index, result);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => bail!("Search pipeline stopped"),
            }
        }
    }

    fn decided(&mut self) {
        self.launch_next();
    }
}

fn run_with_provider_inner(
    settings: Settings,
    provider: &Provider,
    stop: &AtomicBool,
    emit: &impl Fn(Event),
    cache_directory: &Path,
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
    let mut stats = DownloadStats {
        names: HashMap::new(),
        saved: 0,
        skipped: 0,
        no_match: 0,
        failed: 0,
        completed: 0,
        total,
    };
    std::thread::scope(|scope| -> Result<()> {
        let mut searches = SearchPipeline::new(scope, provider, &paths, &settings, stop);
        let mut downloads =
            DownloadPipeline::new(scope, provider, &settings, stop, cache_directory);
        for (index, path) in paths.iter().enumerate() {
            cancelled(stop)?;
            let arcade_mode = Arc::new(AtomicBool::new(settings.arcade_mode));
            // Preserve the relative directories AND extension to prevent same-stem collisions.
            let relative = path.strip_prefix(&settings.root)?;
            let output = image_path(&settings, path)?;
            if output.exists() && !settings.overwrite {
                let _ = searches.take(index)?;
                searches.decided();
                stats.skipped += 1;
                stats.completed += 1;
                emit(Event::Log(format!("Kept: {}", relative.display())));
                emit(Event::Progress(stats.completed, total));
            } else {
                let initial_search = searches.take(index)?;
                emit(Event::Log(format!("Matching: {}", relative.display())));
                let decided = Cell::new(false);
                let mut on_decided = || {
                    if !decided.replace(true) {
                        searches.decided();
                    }
                };
                let result = resolve_with_initial_search(
                    provider,
                    path,
                    ResolveOptions {
                        auto_first_result: settings.auto_first_result,
                        choose_artwork: settings.choose_artwork,
                        kind: settings.kind,
                        width: settings.width,
                        height: settings.height,
                        arcade_root: Some(settings.root.clone()),
                        arcade_mode: arcade_mode.clone(),
                    },
                    initial_search,
                    &mut on_decided,
                    stop,
                    &emit,
                );
                if !decided.get() {
                    on_decided();
                }
                drop(on_decided);
                match result {
                    Ok(Some(Resolved::NoMatch)) => {
                        stats.no_match += 1;
                        stats.completed += 1;
                        emit(Event::Log(format!("No match: {}", relative.display())));
                        emit(Event::Progress(stats.completed, total));
                    }
                    Ok(Some(resolved)) => {
                        while downloads.is_full() {
                            let result = downloads.take()?;
                            if let Some(job) = finish_download(result, stop, emit, &mut stats)? {
                                downloads.spawn(job);
                            }
                        }
                        downloads.spawn(DownloadJob {
                            path: path.clone(),
                            output,
                            relative: relative.display().to_string(),
                            resolved,
                        });
                    }
                    Ok(None) => {
                        stats.skipped += 1;
                        stats.completed += 1;
                        emit(Event::Log(format!("Skipped: {}", relative.display())));
                        emit(Event::Progress(stats.completed, total));
                    }
                    Err(e) => {
                        cancelled(stop)?;
                        if e.downcast_ref::<ServiceStopped>().is_some() {
                            return Err(e);
                        }
                        stats.failed += 1;
                        stats.completed += 1;
                        emit(Event::Log(format!("Review {}: {e}", relative.display())));
                        emit(Event::Progress(stats.completed, total));
                    }
                }
            }
        }
        while downloads.active > 0 {
            let result = downloads.take()?;
            if let Some(job) = finish_download(result, stop, emit, &mut stats)? {
                downloads.spawn(job);
            }
        }
        Ok(())
    })?;
    if settings.gamelist {
        write_gamelist(&settings, &all_paths, &stats.names, &emit)?;
    }
    emit(Event::Summary {
        saved: stats.saved,
        skipped: stats.skipped,
        no_match: stats.no_match,
        failed: stats.failed,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::save_atomic;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::atomic::AtomicUsize,
        thread,
    };

    fn test_credentials() -> Credentials {
        Credentials {
            api_key: "test".into(),
        }
    }

    #[test]
    fn search_pipeline_runs_up_to_three_queries_and_slides_after_a_decision() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let server = thread::spawn({
            let active = active.clone();
            let max_active = max_active.clone();
            move || {
                let mut requests = Vec::new();
                let mut handlers = Vec::new();
                for _ in 0..6 {
                    let (mut stream, _) = listener.accept().unwrap();
                    let mut request = Vec::new();
                    let mut byte = [0];
                    while !request.ends_with(b"\r\n\r\n") {
                        stream.read_exact(&mut byte).unwrap();
                        request.push(byte[0]);
                    }
                    requests.push(String::from_utf8(request).unwrap());
                    let active = active.clone();
                    let max_active = max_active.clone();
                    handlers.push(thread::spawn(move || {
                        let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                        max_active.fetch_max(current, Ordering::SeqCst);
                        thread::sleep(Duration::from_millis(100));
                        let body = r#"{"success":true,"data":[{"id":1,"name":"Game"}]}"#;
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        )
                        .unwrap();
                        active.fetch_sub(1, Ordering::SeqCst);
                    }));
                }
                for handler in handlers {
                    handler.join().unwrap();
                }
                requests
            }
        });
        let paths = (1..=6)
            .map(|index| PathBuf::from(format!("/roms/Game{index}.nes")))
            .collect::<Vec<_>>();
        let settings = Settings {
            root: "/roms".into(),
            output: "/artwork".into(),
            ..Settings::default()
        };
        let provider = Provider::with_base(test_credentials(), &base).unwrap();
        let stop = AtomicBool::new(false);

        std::thread::scope(|scope| {
            let mut pipeline = SearchPipeline::new(scope, &provider, &paths, &settings, &stop);
            for index in 0..paths.len() {
                assert!(matches!(pipeline.take(index).unwrap(), Some(Ok(_))));
                pipeline.decided();
            }
        });

        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 6);
        assert!((2..=DEFAULT_CONCURRENCY).contains(&max_active.load(Ordering::SeqCst)));
        for index in 1..=6 {
            assert!(
                requests
                    .iter()
                    .any(|request| request.contains(&format!("Game{index}")))
            );
        }
    }

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
