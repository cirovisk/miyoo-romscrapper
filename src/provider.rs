use crate::rom::cancelled;
use anyhow::{Context, Result, bail};
use reqwest::{Url, blocking::Client};
use serde::Deserialize;
use serde_json::Value;
use std::{io::Read, sync::atomic::AtomicBool, time::Duration};

#[derive(Clone, Default)]
pub struct Credentials {
    pub api_key: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Candidate {
    pub id: u64,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Media {
    pub url: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub score: i64,
}

#[derive(Debug)]
pub struct ServiceStopped(pub u16);
impl std::fmt::Display for ServiceStopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SteamGridDB stopped (HTTP {}): check your API key, quota or service availability",
            self.0
        )
    }
}
impl std::error::Error for ServiceStopped {}

pub struct Provider {
    client: Client,
    base: Url,
    credentials: Credentials,
}
impl Provider {
    pub fn new(credentials: Credentials) -> Result<Self> {
        Self::with_base(credentials, "https://www.steamgriddb.com/api/v2/")
    }
    pub fn with_base(credentials: Credentials, base: &str) -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent("rom-art-rs/0.2")
                .build()?,
            base: Url::parse(base)?,
            credentials,
        })
    }
    fn api(&self, parts: &[&str], filters: &[(&str, &str)], stop: &AtomicBool) -> Result<Value> {
        if self.credentials.api_key.trim().is_empty() {
            bail!("Enter your personal SteamGridDB API key on the Connection page");
        }
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("Invalid base URL"))?
            .pop_if_empty()
            .extend(parts);
        url.query_pairs_mut().extend_pairs(filters.iter().copied());
        let bytes = self.request(url, true, 16 * 1024 * 1024, stop)?;
        let value: Value =
            serde_json::from_slice(&bytes).context("Invalid SteamGridDB response")?;
        if value["success"] != true {
            bail!("SteamGridDB did not confirm the request");
        }
        Ok(value["data"].clone())
    }
    pub fn search(&self, title: &str, stop: &AtomicBool) -> Result<Vec<Candidate>> {
        if title.trim().is_empty() {
            bail!("Search title is empty");
        }
        Ok(serde_json::from_value(self.api(
            &["search", "autocomplete", title],
            &[],
            stop,
        )?)?)
    }
    pub fn artwork(
        &self,
        game: u64,
        kind: &str,
        width: u32,
        height: u32,
        stop: &AtomicBool,
    ) -> Result<Media> {
        if !matches!(kind, "grids" | "heroes" | "logos" | "icons") {
            bail!("Invalid artwork type");
        }
        choose_media(self.artwork_options(game, kind, stop)?, width, height)
    }
    pub fn artwork_options(&self, game: u64, kind: &str, stop: &AtomicBool) -> Result<Vec<Media>> {
        if !matches!(kind, "grids" | "heroes" | "logos" | "icons") {
            bail!("Invalid artwork type");
        }
        let value = self.api(
            &[kind, "game", &game.to_string()],
            &[
                ("types", "static"),
                ("mimes", "image/png,image/jpeg,image/webp"),
            ],
            stop,
        )?;
        let media: Vec<Media> = serde_json::from_value(value)?;
        if media.is_empty() {
            bail!("No static artwork available for this type");
        }
        Ok(media)
    }
    pub fn get(&self, url: &str, limit: u64, stop: &AtomicBool) -> Result<Vec<u8>> {
        // Never attach the API token to artwork/CDN requests.
        self.request(
            Url::parse(url).context("Invalid artwork URL")?,
            false,
            limit,
            stop,
        )
    }
    fn request(
        &self,
        url: Url,
        authenticated: bool,
        limit: u64,
        stop: &AtomicBool,
    ) -> Result<Vec<u8>> {
        cancelled(stop)?;
        if url.scheme() != "https"
            && !(url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1" | "localhost")))
        {
            bail!("The connection must use HTTPS");
        }
        let mut request = self.client.get(url);
        if authenticated {
            request = request.bearer_auth(self.credentials.api_key.trim());
        }
        let response = request.send().map_err(|e| e.without_url())?;
        let status = response.status();
        if matches!(status.as_u16(), 401 | 403 | 429) || status.is_server_error() {
            return Err(ServiceStopped(status.as_u16()).into());
        }
        if !status.is_success() {
            bail!("Resource unavailable (HTTP {})", status.as_u16());
        }
        let mut bytes = Vec::new();
        response
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .context("Could not receive resource")?;
        cancelled(stop)?;
        if bytes.len() as u64 > limit {
            bail!("Resource exceeds the size limit");
        }
        Ok(bytes)
    }
}

pub fn choose_media(mut media: Vec<Media>, width: u32, height: u32) -> Result<Media> {
    // Prefer matching orientation, then community score; preserve API order on ties.
    media.sort_by_key(|m| {
        (
            (m.width > m.height) != (width > height),
            std::cmp::Reverse(m.score),
        )
    });
    media
        .into_iter()
        .next()
        .context("No static artwork available for this type")
}

pub fn exact_match<'a>(title: &str, candidates: &'a [Candidate]) -> Option<&'a Candidate> {
    let key = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let title = key(title);
    let mut matches = candidates.iter().filter(|c| key(&c.name) == title);
    let first = matches.next()?;
    if matches.next().is_some() {
        None
    } else {
        Some(first)
    }
}
