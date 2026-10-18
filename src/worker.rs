use crate::{
    artwork::decode,
    provider::{ArtworkType, Candidate, Media, Provider},
    rom::{cancelled, title},
};
use anyhow::{Result, bail};
use image::ImageFormat;
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, mpsc},
    time::Duration,
};

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
pub struct ResolveOptions {
    pub auto_first_result: bool,
    pub choose_artwork: bool,
    pub kind: ArtworkType,
    pub width: u32,
    pub height: u32,
}

pub fn resolve(
    provider: &Provider,
    path: &Path,
    options: ResolveOptions,
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
