use crate::engine::save_with_backup;
use anyhow::{Context, Result, bail};
use std::{
    collections::HashMap,
    fs,
    path::{Component, Path, PathBuf},
};
use xmltree::{Element, EmitterConfig, XMLNode};

pub struct Entry {
    pub rom: PathBuf,
    pub name: String,
    pub image: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    EmulationStation,
    OnionOS,
    Both,
}
impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Self::EmulationStation => "gamelist.xml",
            Self::OnionOS => "miyoogamelist.xml",
            Self::Both => "gamelist.xml + miyoogamelist.xml",
        }
    }
}

pub fn export(
    root: &Path,
    entries: &[Entry],
    backup: bool,
    format: Format,
) -> Result<Vec<PathBuf>> {
    // Validate both documents before updating either file.
    let files: &[&str] = match format {
        Format::EmulationStation => &["gamelist.xml"],
        Format::OnionOS | Format::Both => &["gamelist.xml", "miyoogamelist.xml"],
    };
    for filename in files {
        let bytes = match fs::read(root.join(filename)) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        if let Some(bytes) = bytes {
            let document = Element::parse(bytes.as_slice())
                .with_context(|| format!("Invalid {filename}; file preserved"))?;
            if document.name != "gameList" {
                bail!("Invalid root in {filename}; expected gameList");
            }
        }
    }
    let mut backups = Vec::new();
    if format != Format::OnionOS
        && let Some(path) = update(root, entries, backup)?
    {
        backups.push(path);
    }
    if format != Format::EmulationStation
        && let Some(path) = update_file(root, entries, backup, true)?
    {
        backups.push(path);
    }
    Ok(backups)
}

fn text(element: &Element, key: &str) -> Option<String> {
    element.get_child(key)?.get_text().map(|s| s.into_owned())
}
fn set(element: &mut Element, key: &str, value: &str) {
    if let Some(child) = element.get_mut_child(key) {
        child.children = vec![XMLNode::Text(value.into())];
    } else {
        let mut child = Element::new(key);
        child.children.push(XMLNode::Text(value.into()));
        element.children.push(XMLNode::Element(child));
    }
}
fn normalized(root: &Path, path: &Path) -> PathBuf {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            _ => result.push(component),
        }
    }
    result
}
fn xml_path(root: &Path, path: &Path) -> Result<String> {
    let path = normalized(root, path);
    let value = if let Ok(relative) = path.strip_prefix(root) {
        format!("./{}", relative.to_str().context("Path is not UTF-8")?)
    } else {
        path.to_str().context("Path is not UTF-8")?.to_owned()
    };
    Ok(value)
}

/// Preserves existing game/folder records and metadata; never deletes stale records.
pub fn update(root: &Path, entries: &[Entry], backup_existing: bool) -> Result<Option<PathBuf>> {
    update_file(root, entries, backup_existing, false)
}

fn update_file(
    root: &Path,
    entries: &[Entry],
    backup_existing: bool,
    onion: bool,
) -> Result<Option<PathBuf>> {
    let root = root.canonicalize()?;
    let path = root.join(if onion {
        "miyoogamelist.xml"
    } else {
        "gamelist.xml"
    });
    let original = match fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let source = if onion && original.is_none() {
        match fs::read(root.join("gamelist.xml")) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        }
    } else {
        None
    };
    let mut document = if let Some(bytes) = original.as_ref().or(source.as_ref()) {
        Element::parse(bytes.as_slice()).context("Invalid game list XML; file preserved")?
    } else {
        Element::new("gameList")
    };
    if document.name != "gameList" {
        bail!("Existing XML has no gameList root; file preserved");
    }
    let mut indices = HashMap::new();
    for (index, node) in document.children.iter().enumerate() {
        if let XMLNode::Element(game) = node
            && game.name == "game"
            && let Some(value) = text(game, "path")
        {
            let value = if let Some(rest) = value.strip_prefix("~/") {
                PathBuf::from(std::env::var_os("HOME").context("HOME is unavailable")?).join(rest)
            } else {
                PathBuf::from(value)
            };
            if indices.insert(normalized(&root, &value), index).is_some() {
                bail!("Game list contains duplicate paths; review before updating");
            }
        }
    }
    let mut changed = original.is_none();
    for entry in entries {
        let rom = entry
            .rom
            .canonicalize()
            .context("Game list ROM does not exist")?;
        if !rom.starts_with(&root) {
            bail!("ROM is outside the system folder");
        }
        let index = if let Some(index) = indices.get(&rom) {
            *index
        } else {
            let mut game = Element::new("game");
            set(&mut game, "path", &xml_path(&root, &rom)?);
            document.children.push(XMLNode::Element(game));
            let index = document.children.len() - 1;
            indices.insert(rom, index);
            changed = true;
            index
        };
        let XMLNode::Element(game) = &mut document.children[index] else {
            unreachable!()
        };
        // Retain names customized in the frontend.
        if text(game, "name").is_none_or(|name| name.trim().is_empty()) {
            set(game, "name", &entry.name);
            changed = true;
        }
        if let Some(image) = &entry.image {
            let image = image
                .canonicalize()
                .context("Game list artwork does not exist")?;
            let value = xml_path(&root, &image)?;
            if text(game, "image").as_deref() != Some(&value) {
                set(game, "image", &value);
                changed = true;
            }
        }
    }
    if onion {
        changed |= prepare_onion(&root, &mut document)?;
    }
    if !changed {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    document.write_with_config(&mut bytes, EmitterConfig::new().perform_indent(!onion))?;
    Element::parse(bytes.as_slice()).context("Could not validate the generated game list")?;
    let current = match fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    if current != original {
        bail!("Game list changed during export; run again");
    }
    save_with_backup(&path, &bytes, true, backup_existing)
}

fn prepare_onion(root: &Path, document: &mut Element) -> Result<bool> {
    let mut changed = false;
    let before = document.children.len();
    document
        .children
        .retain(|node| matches!(node, XMLNode::Element(game) if game.name == "game"));
    changed |= document.children.len() != before;
    for node in &mut document.children {
        let XMLNode::Element(game) = node else {
            unreachable!()
        };
        let before = game.children.len();
        game.children.retain(|node| matches!(node, XMLNode::Element(field) if matches!(field.name.as_str(),"path"|"name"|"image")));
        changed |= game.children.len() != before;
        let rom = text(game, "path").context("OnionOS entry has no ROM path")?;
        let rom = normalized(root, Path::new(&rom));
        if !rom.starts_with(root) {
            bail!("OnionOS ROM paths must stay inside the system folder");
        }
        let value = xml_path(root, &rom)?;
        if text(game, "path").as_deref() != Some(&value) {
            set(game, "path", &value);
            changed = true;
        }
        if text(game, "name").is_none_or(|s| s.trim().is_empty()) {
            set(game, "name", &crate::rom::title(&rom)?);
            changed = true;
        }
        let image = text(game, "image")
            .filter(|s| !s.trim().is_empty())
            .map(|s| normalized(root, Path::new(&s)));
        let image = if let Some(image) =
            image.filter(|path| path.is_file() && image::image_dimensions(path).is_ok())
        {
            if !image.starts_with(root) {
                bail!(
                    "OnionOS images must be inside the system folder; select an OnionOS artwork preset"
                );
            }
            image
        } else {
            let image = root.join("Imgs/.rom-art-placeholder.png");
            if !image.exists() {
                let mut png = std::io::Cursor::new(Vec::new());
                image::DynamicImage::new_rgba8(1, 1).write_to(&mut png, image::ImageFormat::Png)?;
                save_with_backup(&image, png.get_ref(), false, false)?;
            } else if image::image_dimensions(&image).is_err() {
                bail!("Invalid OnionOS placeholder: {}", image.display());
            }
            image
        };
        let value = xml_path(root, &image)?;
        if text(game, "image").as_deref() != Some(&value) {
            set(game, "image", &value);
            changed = true;
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn onion_export_retains_source_metadata_and_provides_real_images() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let rom = root.join("A & B.gba");
        fs::write(&rom, b"rom").unwrap();
        let original = b"<gameList><game><path>./A &amp; B.gba</path><name>Custom title</name><desc>Keep this</desc><playcount>9</playcount></game><game><path>./Old.gba</path><name>Old</name></game></gameList>";
        fs::write(root.join("gamelist.xml"), original).unwrap();
        let entries = vec![Entry {
            rom: rom.clone(),
            name: "New title".into(),
            image: None,
        }];
        export(root, &entries, true, Format::OnionOS).unwrap();
        assert_eq!(fs::read(root.join("gamelist.xml")).unwrap(), original);
        let bytes = fs::read(root.join("miyoogamelist.xml")).unwrap();
        assert!(!bytes.contains(&b'\n'));
        let document = Element::parse(bytes.as_slice()).unwrap();
        assert_eq!(
            text(document.get_child("game").unwrap(), "name").unwrap(),
            "Custom title"
        );
        for node in &document.children {
            let XMLNode::Element(game) = node else {
                panic!("Unexpected node");
            };
            assert_eq!(game.children.len(), 3);
            assert!(game.get_child("desc").is_none());
            let image = text(game, "image").unwrap();
            assert_eq!(image::image_dimensions(root.join(image)).unwrap(), (1, 1));
        }
        assert!(
            export(root, &entries, true, Format::OnionOS)
                .unwrap()
                .is_empty()
        );
        assert_eq!(fs::read(root.join("miyoogamelist.xml")).unwrap(), bytes);
        let image = root.join("Imgs/A & B.png");
        image::DynamicImage::new_rgba8(250, 360)
            .save(&image)
            .unwrap();
        let backup = export(
            root,
            &[Entry {
                rom,
                name: "New title".into(),
                image: Some(image),
            }],
            true,
            Format::OnionOS,
        )
        .unwrap();
        assert_eq!(backup.len(), 1);
        assert_eq!(fs::read(&backup[0]).unwrap(), bytes);
    }

    #[test]
    fn both_formats_keep_full_metadata_only_in_standard_list() {
        let dir = tempfile::tempdir().unwrap();
        let rom = dir.path().join("Game.nes");
        fs::write(&rom, b"rom").unwrap();
        fs::write(dir.path().join("gamelist.xml"),b"<gameList><game><path>./Game.nes</path><name>Game</name><desc>Metadata</desc></game></gameList>").unwrap();
        export(
            dir.path(),
            &[Entry {
                rom,
                name: "Game".into(),
                image: None,
            }],
            true,
            Format::Both,
        )
        .unwrap();
        for (filename, expected) in [("gamelist.xml", true), ("miyoogamelist.xml", false)] {
            let bytes = fs::read(dir.path().join(filename)).unwrap();
            let doc = Element::parse(bytes.as_slice()).unwrap();
            assert_eq!(
                doc.get_child("game").unwrap().get_child("desc").is_some(),
                expected
            );
        }
    }

    #[test]
    fn invalid_onion_list_is_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("miyoogamelist.xml"), b"<broken>").unwrap();
        assert!(export(dir.path(), &[], true, Format::Both).is_err());
        assert!(!dir.path().join("gamelist.xml").exists());
        assert_eq!(
            fs::read(dir.path().join("miyoogamelist.xml")).unwrap(),
            b"<broken>"
        );
    }
    #[test]
    fn backup_can_be_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let rom = dir.path().join("Game.nes");
        fs::write(&rom, b"rom").unwrap();
        fs::write(dir.path().join("gamelist.xml"), b"<gameList/>").unwrap();
        assert!(
            update(
                dir.path(),
                &[Entry {
                    rom,
                    name: "Game".into(),
                    image: None
                }],
                false
            )
            .unwrap()
            .is_none()
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
        let bytes = fs::read(dir.path().join("gamelist.xml")).unwrap();
        assert!(
            Element::parse(bytes.as_slice())
                .unwrap()
                .get_child("game")
                .is_some()
        );
    }
    #[test]
    fn merges_escapes_preserves_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let rom = root.join("A & B.nes");
        fs::write(&rom, b"rom").unwrap();
        fs::create_dir(root.join("Imgs")).unwrap();
        let image = root.join("Imgs/A & B.png");
        fs::write(&image, b"image").unwrap();
        let original = b"<gameList><game custom=\"yes\"><path>./A &amp; B.nes</path><name>Meu nome</name><desc>Descricao</desc><playcount>42</playcount><favorite>true</favorite></game><game><path>./ausente.nes</path><name>Manter</name></game><folder><path>./pasta</path><name>Pasta</name></folder></gameList>";
        fs::write(root.join("gamelist.xml"), original).unwrap();
        let entries = vec![Entry {
            rom,
            image: Some(image),
            name: "Outro nome".into(),
        }];
        let backup = update(root, &entries, true).unwrap().unwrap();
        assert_eq!(fs::read(backup).unwrap(), original);
        let bytes = fs::read(root.join("gamelist.xml")).unwrap();
        let doc = Element::parse(bytes.as_slice()).unwrap();
        let game = doc.get_child("game").unwrap();
        assert_eq!(text(game, "name").unwrap(), "Meu nome");
        assert_eq!(text(game, "playcount").unwrap(), "42");
        assert_eq!(text(game, "image").unwrap(), "./Imgs/A & B.png");
        assert_eq!(game.attributes["custom"], "yes");
        assert_eq!(
            doc.children
                .iter()
                .filter(|n| matches!(n, XMLNode::Element(_)))
                .count(),
            3
        );
        assert!(update(root, &entries, true).unwrap().is_none());
        assert_eq!(fs::read(root.join("gamelist.xml")).unwrap(), bytes);
    }
    #[test]
    fn malformed_xml_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gamelist.xml");
        fs::write(&path, b"<broken>").unwrap();
        assert!(update(dir.path(), &[], true).is_err());
        assert_eq!(fs::read(path).unwrap(), b"<broken>");
    }
    #[test]
    fn new_list_includes_roms_without_images() {
        let dir = tempfile::tempdir().unwrap();
        let rom = dir.path().join("Novo.nes");
        fs::write(&rom, b"rom").unwrap();
        update(
            dir.path(),
            &[Entry {
                rom,
                name: "A & B <Novo>".into(),
                image: None,
            }],
            true,
        )
        .unwrap();
        let bytes = fs::read(dir.path().join("gamelist.xml")).unwrap();
        let doc = Element::parse(bytes.as_slice()).unwrap();
        let game = doc.get_child("game").unwrap();
        assert_eq!(text(game, "name").unwrap(), "A & B <Novo>");
        assert!(game.get_child("image").is_none());
    }
}
