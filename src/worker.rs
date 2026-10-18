use crate::{
    arcade,
    artwork::decode,
    provider::{ArtworkType, Candidate, Media, Provider, exact_match},
    rom::{cancelled, title},
};
use anyhow::{Result, bail};
use image::ImageFormat;
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

pub enum Event {
    Log(String),
    Summary {
        saved: usize,
        skipped: usize,
        no_match: usize,
        failed: usize,
    },
    Progress(usize, usize),
    SystemProgress(usize, usize, Option<String>),
    Preview(PathBuf),
    Finished,
    Review {
        rom: PathBuf,
        query: String,
        candidates: Vec<Candidate>,
        arcade: bool,
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
    SetArcade(bool),
    LocalImage(PathBuf),
    SelectArtwork(Media),
    Skip,
}

#[derive(Debug)]
pub enum Resolved {
    Game(Candidate, Option<Media>),
    LocalImage(PathBuf),
    NoMatch,
}

#[derive(Clone)]
pub struct ResolveOptions {
    pub auto_first_result: bool,
    pub choose_artwork: bool,
    pub kind: ArtworkType,
    pub width: u32,
    pub height: u32,
    pub arcade_root: Option<PathBuf>,
    pub arcade_mode: Arc<AtomicBool>,
}

pub fn resolve(
    provider: &Provider,
    path: &Path,
    options: ResolveOptions,
    stop: &AtomicBool,
    emit: &impl Fn(Event),
) -> Result<Option<Resolved>> {
    let mut on_decided = || {};
    resolve_with_initial_search(provider, path, options, None, &mut on_decided, stop, emit)
}

pub fn resolve_with_initial_search(
    provider: &Provider,
    path: &Path,
    options: ResolveOptions,
    mut initial_search: Option<std::result::Result<Vec<Candidate>, String>>,
    on_decided: &mut impl FnMut(),
    stop: &AtomicBool,
    emit: &impl Fn(Event),
) -> Result<Option<Resolved>> {
    let mut arcade_mode = options.arcade_mode.load(Ordering::Relaxed);
    let mut query = initial_search_query(path, options.arcade_root.as_deref(), arcade_mode)?;
    loop {
        let candidates = match initial_search.take() {
            Some(Ok(candidates)) => candidates,
            Some(Err(error)) => return Err(anyhow::anyhow!(error)),
            None => provider.search(&query, stop)?,
        };
        if options.auto_first_result {
            if let Some(candidate) = exact_match(&query, &candidates) {
                on_decided();
                return choose_game(
                    provider,
                    path,
                    candidate.clone(),
                    options,
                    arcade_mode,
                    stop,
                    emit,
                );
            }
            on_decided();
            return Ok(Some(Resolved::NoMatch));
        }
        let (tx, rx) = mpsc::channel();
        emit(Event::Review {
            rom: path.to_path_buf(),
            query: query.clone(),
            candidates: candidates.clone(),
            arcade: arcade_mode,
            answer: tx,
        });
        loop {
            cancelled(stop)?;
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(ReviewAction::Select(candidate)) if candidates.contains(&candidate) => {
                    on_decided();
                    return choose_game(
                        provider,
                        path,
                        candidate,
                        options,
                        arcade_mode,
                        stop,
                        emit,
                    );
                }
                Ok(ReviewAction::Select(_)) => bail!("Selection is not among the search results"),
                Ok(ReviewAction::Skip) => {
                    on_decided();
                    return if exact_match(&query, &candidates).is_none() {
                        Ok(Some(Resolved::NoMatch))
                    } else {
                        Ok(None)
                    };
                }
                Ok(ReviewAction::LocalImage(path)) => {
                    on_decided();
                    return Ok(Some(Resolved::LocalImage(path)));
                }
                Ok(ReviewAction::SelectArtwork(_)) => {
                    bail!("Choose a game before choosing artwork")
                }
                Ok(ReviewAction::Search(search)) => {
                    query = search;
                    break;
                }
                Ok(ReviewAction::SetArcade(enabled)) => {
                    arcade_mode = enabled;
                    options.arcade_mode.store(enabled, Ordering::Relaxed);
                    query = initial_search_query(path, options.arcade_root.as_deref(), enabled)?;
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => bail!("Review closed"),
            }
        }
    }
}

pub fn initial_search_query(
    path: &Path,
    arcade_root: Option<&Path>,
    arcade_mode: bool,
) -> Result<String> {
    if arcade_mode {
        Ok(arcade_root
            .and_then(|root| arcade::title(root, path).ok().flatten())
            .unwrap_or_else(|| {
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            }))
    } else {
        title(path)
    }
}

fn choose_game(
    provider: &Provider,
    path: &Path,
    candidate: Candidate,
    options: ResolveOptions,
    arcade_mode: bool,
    stop: &AtomicBool,
    emit: &impl Fn(Event),
) -> Result<Option<Resolved>> {
    if arcade_mode
        && let Some(root) = options.arcade_root.as_deref()
        && let Err(error) = arcade::remember(root, path, &candidate.name)
    {
        emit(Event::Log(format!(
            "Could not save arcade title alias: {error}"
        )));
    }
    resolve_artwork(provider, path, candidate, options, stop, emit)
}

fn resolve_artwork(
    provider: &Provider,
    rom: &Path,
    game: Candidate,
    options: ResolveOptions,
    stop: &AtomicBool,
    emit: &impl Fn(Event),
) -> Result<Option<Resolved>> {
    if !options.choose_artwork {
        return Ok(Some(Resolved::Game(game, None)));
    }
    let mut media = match provider.artwork_options(game.id, options.kind, stop) {
        Ok(media) => media,
        Err(error) => return request_local_image(rom, &error.to_string(), stop, emit),
    };
    media.sort_by_key(|m| {
        (
            (m.width > m.height) != (options.width > options.height),
            std::cmp::Reverse(m.score),
        )
    });
    let previews = std::thread::scope(|scope| {
        media
            .chunks(4)
            .flat_map(|chunk| {
                chunk
                    .iter()
                    .map(|item| scope.spawn(move || artwork_preview(provider, item, stop)))
                    .collect::<Vec<_>>()
                    .into_iter()
                    .map(|thread| thread.join().ok().flatten())
            })
            .collect::<Vec<_>>()
    });
    cancelled(stop)?;
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

fn artwork_preview(provider: &Provider, media: &Media, stop: &AtomicBool) -> Option<Vec<u8>> {
    let url = media.thumb.as_deref().unwrap_or(&media.url);
    let bytes = provider.get(url, 4 * 1024 * 1024, stop).ok()?;
    let thumbnail = decode(&bytes).ok()?.thumbnail(150, 200);
    let mut output = Cursor::new(Vec::new());
    thumbnail.write_to(&mut output, ImageFormat::Jpeg).ok()?;
    Some(output.into_inner())
}

pub fn request_local_image(
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
        arcade: false,
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
