use crate::{
    provider::{ArtworkType, Candidate, Media, Provider},
    storage::save_atomic,
};
use anyhow::Result;
use image::{
    DynamicImage, RgbaImage,
    imageops::{FilterType, overlay},
};
use sha1::{Digest, Sha1};
use std::{fs, io::Cursor, path::Path, sync::atomic::AtomicBool};

pub struct Options<'a> {
    pub kind: ArtworkType,
    pub width: u32,
    pub height: u32,
    pub cache_directory: &'a Path,
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
    kind: ArtworkType,
    selected: Option<&Media>,
    options: &Options<'_>,
    stop: &AtomicBool,
) -> Result<DynamicImage> {
    let (width, height) = match kind {
        ArtworkType::Hero => (1920, 620),
        _ => (options.width, options.height),
    };
    let media = match selected {
        Some(media) => media.clone(),
        None => provider.artwork(game.id, kind, width, height, stop)?,
    };
    let key = format!("{:x}", Sha1::digest(media.url.as_bytes()));
    let cache = options.cache_directory.join(key);
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

pub fn decode(bytes: &[u8]) -> Result<DynamicImage> {
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
    options: &Options<'_>,
    selected: Option<&Media>,
    stop: &AtomicBool,
) -> Result<DynamicImage> {
    Ok(fit(
        &fetch_image(provider, game, options.kind, selected, options, stop)?,
        options.width,
        options.height,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn decode_rejects_non_image_data() {
        assert!(decode(b"not an image").is_err());
    }
}
