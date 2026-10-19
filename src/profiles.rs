use crate::collection_ui::SystemFolder;
use rom_art_rs::{
    engine::Settings, gamelist::Format, provider::ArtworkType, storage::app_data_dir,
};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct DeviceProfile {
    pub name: String,
    pub roots: Vec<SystemFolder>,
    pub output: String,
    pub local_artwork_folder: String,
    pub settings: ProfileSettings,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct ProfileSettings {
    extensions: String,
    kind: ArtworkType,
    width: u32,
    height: u32,
    overwrite: bool,
    backup_existing: bool,
    auto_first_result: bool,
    turbo_mode: bool,
    concurrency: usize,
    choose_artwork: bool,
    only_missing: bool,
    #[serde(default)]
    only_new_or_changed: bool,
    gamelist: bool,
    onion_images: bool,
    gamelist_format: Format,
}

impl ProfileSettings {
    fn from_settings(settings: &Settings) -> Self {
        Self {
            extensions: settings.extensions.clone(),
            kind: settings.kind,
            width: settings.width,
            height: settings.height,
            overwrite: settings.overwrite,
            backup_existing: settings.backup_existing,
            auto_first_result: settings.auto_first_result,
            turbo_mode: settings.turbo_mode,
            concurrency: settings.concurrency,
            choose_artwork: settings.choose_artwork,
            only_missing: settings.only_missing,
            only_new_or_changed: settings.only_new_or_changed,
            gamelist: settings.gamelist,
            onion_images: settings.onion_images,
            gamelist_format: settings.gamelist_format,
        }
    }

    fn apply_to(&self, settings: &mut Settings) {
        settings.extensions = self.extensions.clone();
        settings.kind = self.kind;
        settings.width = self.width;
        settings.height = self.height;
        settings.overwrite = self.overwrite;
        settings.backup_existing = self.backup_existing;
        settings.auto_first_result = self.auto_first_result;
        settings.turbo_mode = self.turbo_mode;
        settings.concurrency = self.concurrency;
        settings.choose_artwork = self.choose_artwork;
        settings.only_missing = self.only_missing;
        settings.only_new_or_changed = self.only_new_or_changed;
        settings.gamelist = self.gamelist;
        settings.onion_images = self.onion_images;
        settings.gamelist_format = self.gamelist_format;
    }
}

impl DeviceProfile {
    pub(crate) fn capture(
        name: String,
        roots: &[SystemFolder],
        output: &str,
        local_artwork_folder: &str,
        settings: &Settings,
    ) -> Self {
        Self {
            name,
            roots: roots.to_vec(),
            output: output.to_owned(),
            local_artwork_folder: local_artwork_folder.to_owned(),
            settings: ProfileSettings::from_settings(settings),
        }
    }

    pub(crate) fn apply_to(
        &self,
        roots: &mut Vec<SystemFolder>,
        output: &mut String,
        local_artwork_folder: &mut String,
        settings: &mut Settings,
    ) {
        *roots = self.roots.clone();
        *output = self.output.clone();
        *local_artwork_folder = self.local_artwork_folder.clone();
        self.settings.apply_to(settings);
    }
}

fn profiles_file() -> anyhow::Result<PathBuf> {
    Ok(app_data_dir()?.join("device-profiles.json"))
}

pub(crate) fn load() -> anyhow::Result<Vec<DeviceProfile>> {
    let path = profiles_file()?;
    match fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn save(profiles: &[DeviceProfile]) -> anyhow::Result<()> {
    let path = profiles_file()?;
    let bytes = serde_json::to_vec_pretty(profiles)?;
    rom_art_rs::storage::save_atomic(&path, &bytes, true)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_profiles_preserve_settings_but_never_capture_api_keys() {
        let mut original = Settings::default();
        original.credentials.api_key = "secret-api-key".into();
        original.width = 600;
        let roots = vec![SystemFolder {
            path: "/roms/GB".into(),
            output: String::new(),
            arcade: false,
            system_id: "GB".into(),
        }];
        let profile = DeviceProfile::capture(
            "Miyoo Mini".into(),
            &roots,
            "/artwork",
            "/local-art",
            &original,
        );
        let serialized = serde_json::to_string(&profile).unwrap();
        assert!(!serialized.contains("secret-api-key"));

        let mut restored = Settings::default();
        restored.credentials.api_key = "session-key".into();
        let mut restored_roots = Vec::new();
        let mut output = String::new();
        let mut local_artwork = String::new();
        profile.apply_to(
            &mut restored_roots,
            &mut output,
            &mut local_artwork,
            &mut restored,
        );
        assert_eq!(restored.width, 600);
        assert_eq!(restored.credentials.api_key, "session-key");
        assert_eq!(restored_roots[0].system_id, "GB");
        assert_eq!(output, "/artwork");
        assert_eq!(local_artwork, "/local-art");
    }
}
