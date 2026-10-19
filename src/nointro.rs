use anyhow::{Context, Result, bail};
use sha1::Digest;
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};
use xmltree::{Element, XMLNode};

#[derive(Clone)]
pub struct RenameItem {
    pub source: PathBuf,
    pub target: PathBuf,
}

#[derive(Clone, Default)]
pub struct RenamePreview {
    pub matches: Vec<RenameItem>,
    pub unmatched: Vec<PathBuf>,
    pub unchanged: usize,
    pub conflicts: Vec<(PathBuf, PathBuf)>,
}

#[derive(Clone, Default)]
struct DatIndex {
    crc32: HashMap<String, Vec<String>>,
    sha1: HashMap<String, Vec<String>>,
}

impl DatIndex {
    fn read(path: &Path) -> Result<Self> {
        let bytes =
            fs::read(path).with_context(|| format!("Could not read DAT: {}", path.display()))?;
        let mut index = Self::default();
        if let Ok(root) = Element::parse(bytes.as_slice()) {
            collect_games(&root, &mut index);
        } else if let Ok(source) = std::str::from_utf8(&bytes) {
            collect_clrmamepro(source, &mut index);
        }
        if index.crc32.is_empty() && index.sha1.is_empty() {
            bail!("The DAT contains no ROM CRC32 or SHA-1 checksums");
        }
        Ok(index)
    }

    fn names_for(&self, crc32: &str, sha1: &str) -> Vec<String> {
        if let Some(found) = self.sha1.get(sha1) {
            return found.to_vec();
        }
        self.crc32.get(crc32).cloned().unwrap_or_default()
    }
}

#[derive(Default)]
struct DatFrame {
    kind: String,
    name: Option<String>,
    crc32: Option<String>,
    sha1: Option<String>,
}

fn collect_clrmamepro(source: &str, index: &mut DatIndex) {
    let tokens = crate::arcade::dat_tokens(source);
    let mut stack = Vec::<DatFrame>::new();
    let mut pending = None::<String>;
    for (position, token) in tokens.iter().enumerate() {
        match token.as_str() {
            "(" => {
                if let Some(kind) = position.checked_sub(1).and_then(|p| tokens.get(p)) {
                    stack.push(DatFrame {
                        kind: kind.clone(),
                        ..DatFrame::default()
                    });
                }
                pending = None;
            }
            ")" => {
                if let Some(frame) = stack.pop()
                    && frame.kind == "rom"
                    && let Some(game) = stack.iter().rev().find(|frame| frame.kind == "game")
                    && let Some(name) = game.name.as_ref().filter(|name| safe_title(name))
                {
                    if let Some(crc) = frame.crc32 {
                        index
                            .crc32
                            .entry(crc.to_ascii_lowercase())
                            .or_default()
                            .push(name.clone());
                    }
                    if let Some(sha1) = frame.sha1 {
                        index
                            .sha1
                            .entry(sha1.to_ascii_lowercase())
                            .or_default()
                            .push(name.clone());
                    }
                }
                pending = None;
            }
            "name" | "crc" | "sha1" => pending = Some(token.clone()),
            value => {
                if let Some(key) = pending.take()
                    && let Some(frame) = stack.last_mut()
                {
                    match key.as_str() {
                        "name" if frame.kind == "game" => frame.name = Some(value.to_owned()),
                        "crc" if frame.kind == "rom" => frame.crc32 = Some(value.to_owned()),
                        "sha1" if frame.kind == "rom" => frame.sha1 = Some(value.to_owned()),
                        _ => {}
                    }
                }
            }
        }
    }
}

fn collect_games(element: &Element, index: &mut DatIndex) {
    if matches!(element.name.as_str(), "game" | "machine")
        && let Some(name) = element.attributes.get("name")
        && safe_title(name)
    {
        for rom in element
            .children
            .iter()
            .filter_map(as_element)
            .filter(|child| child.name == "rom")
        {
            if let Some(crc) = rom.attributes.get("crc") {
                index
                    .crc32
                    .entry(crc.to_ascii_lowercase())
                    .or_default()
                    .push(name.clone());
            }
            if let Some(sha1) = rom.attributes.get("sha1") {
                index
                    .sha1
                    .entry(sha1.to_ascii_lowercase())
                    .or_default()
                    .push(name.clone());
            }
        }
    }
    for child in element.children.iter().filter_map(as_element) {
        collect_games(child, index);
    }
}

fn as_element(node: &XMLNode) -> Option<&Element> {
    match node {
        XMLNode::Element(element) => Some(element),
        _ => None,
    }
}

fn safe_title(value: &str) -> bool {
    value.is_ascii()
        && !value.trim().is_empty()
        && value == value.trim()
        && !value.starts_with('.')
        && !value.ends_with('.')
        && !value
            .chars()
            .any(|ch| matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        && value != "."
        && value != ".."
}

pub fn preview(root: &Path, extensions: &str, dat_path: &Path) -> Result<RenamePreview> {
    let index = DatIndex::read(dat_path)?;
    let paths = crate::rom::scan(root, extensions)?;
    let mut preview = RenamePreview::default();
    let mut planned = HashSet::new();
    for source in paths {
        let candidates = hash_file(&source, &index)?;
        let names = candidates
            .iter()
            .flat_map(|(crc, sha1)| index.names_for(crc, sha1))
            .collect::<HashSet<_>>();
        if names.len() != 1 {
            preview.unmatched.push(source);
            continue;
        }
        let title = names.into_iter().next().unwrap();
        let extension = source
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default();
        let target_name = if extension.is_empty() {
            title
        } else {
            format!("{title}.{}", extension.to_ascii_lowercase())
        };
        let target = source.with_file_name(target_name);
        if target == source {
            preview.unchanged += 1;
        } else if target.exists() || !planned.insert(target.clone()) {
            preview.conflicts.push((source, target));
        } else {
            preview.matches.push(RenameItem { source, target });
        }
    }
    Ok(preview)
}

fn hash_file(path: &Path, index: &DatIndex) -> Result<Vec<(String, String)>> {
    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
    {
        let file =
            File::open(path).with_context(|| format!("Could not read {}", path.display()))?;
        let mut archive = zip::ZipArchive::new(file)
            .with_context(|| format!("Could not open ZIP archive {}", path.display()))?;
        let mut matches = Vec::new();
        for item in 0..archive.len() {
            let mut entry = archive.by_index(item)?;
            if entry.is_dir() {
                continue;
            }
            let (crc, sha1) = hashes(&mut entry)?;
            if !index.names_for(&crc, &sha1).is_empty() {
                matches.push((crc, sha1));
            }
        }
        Ok(matches)
    } else {
        let mut file =
            File::open(path).with_context(|| format!("Could not read {}", path.display()))?;
        Ok(vec![hashes(&mut file)?])
    }
}

fn hashes(reader: &mut impl Read) -> Result<(String, String)> {
    let mut crc = crc32fast::Hasher::new();
    let mut sha1 = sha1::Sha1::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        crc.update(&buffer[..read]);
        sha1.update(&buffer[..read]);
    }
    Ok((
        format!("{:08x}", crc.finalize()),
        format!("{:x}", sha1.finalize()),
    ))
}

pub fn apply(preview: &RenamePreview) -> Result<usize> {
    for item in &preview.matches {
        if !item.source.is_file() {
            bail!("ROM no longer exists: {}", item.source.display());
        }
        if item.target.exists() {
            bail!("Rename target already exists: {}", item.target.display());
        }
    }
    let mut completed: Vec<&RenameItem> = Vec::new();
    for item in &preview.matches {
        if let Err(error) = fs::rename(&item.source, &item.target) {
            let mut rollback_errors = Vec::new();
            for done in completed.into_iter().rev() {
                if let Err(rollback) = fs::rename(&done.target, &done.source) {
                    rollback_errors.push(format!(
                        "{} back to {}: {rollback}",
                        done.target.display(),
                        done.source.display()
                    ));
                }
            }
            let error = anyhow::Error::new(error).context(format!(
                "Could not rename {} to {}",
                item.source.display(),
                item.target.display()
            ));
            if rollback_errors.is_empty() {
                return Err(error);
            }
            bail!(
                "{error}; rollback also failed for {}",
                rollback_errors.join("; ")
            );
        }
        completed.push(item);
    }
    Ok(completed.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn previews_exact_raw_rom_and_zip_matches_and_leaves_unknown_roms_unmatched() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("roms");
        fs::create_dir(&root).unwrap();
        let raw = b"raw game bytes";
        fs::write(root.join("totally-different.nes"), raw).unwrap();
        fs::write(root.join("unknown.nes"), b"unknown data").unwrap();
        let mut zip_bytes = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut zip_bytes));
            zip.start_file("internal.bin", zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"zip game bytes").unwrap();
            zip.finish().unwrap();
        }
        fs::write(root.join("mystery.zip"), zip_bytes).unwrap();
        let raw_crc = format!("{:08x}", crc32fast::hash(raw));
        let raw_sha1 = format!("{:x}", sha1::Sha1::digest(raw));
        let zip_crc = format!("{:08x}", crc32fast::hash(b"zip game bytes"));
        let zip_sha1 = format!("{:x}", sha1::Sha1::digest(b"zip game bytes"));
        let dat = temp.path().join("system.dat");
        fs::write(
            &dat,
            format!("<datafile><game name=\"Known Raw (USA)\"><rom crc=\"{raw_crc}\" sha1=\"{raw_sha1}\"/></game><game name=\"Known Zip (USA)\"><rom crc=\"{zip_crc}\" sha1=\"{zip_sha1}\"/></game></datafile>"),
        )
        .unwrap();

        let preview = preview(&root, "nes,zip", &dat).unwrap();
        assert_eq!(preview.matches.len(), 2);
        assert_eq!(preview.unmatched, vec![root.join("unknown.nes")]);
        assert!(
            preview
                .matches
                .iter()
                .any(|item| item.target.file_name().unwrap() == "Known Raw (USA).nes")
        );
        assert!(
            preview
                .matches
                .iter()
                .any(|item| item.target.file_name().unwrap() == "Known Zip (USA).zip")
        );
    }

    #[test]
    fn refuses_unsafe_names_and_preexisting_targets() {
        assert!(!safe_title("../outside"));
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let source = root.join("wrong.nes");
        let target = root.join("Canonical.nes");
        fs::write(&source, b"source").unwrap();
        fs::write(&target, b"keep").unwrap();
        let preview = RenamePreview {
            matches: vec![RenameItem {
                source: source.clone(),
                target: target.clone(),
            }],
            ..RenamePreview::default()
        };
        assert!(apply(&preview).is_err());
        assert_eq!(fs::read(&source).unwrap(), b"source");
        assert_eq!(fs::read(&target).unwrap(), b"keep");
    }

    #[test]
    fn reads_clrmamepro_dat_entries() {
        let mut index = DatIndex::default();
        collect_clrmamepro(
            r#"game ( name "Canonical Game (USA)" rom ( name "source.nes" crc "a1b2c3d4" sha1 "001122" ) )"#,
            &mut index,
        );
        assert_eq!(
            index.names_for("a1b2c3d4", "001122"),
            vec!["Canonical Game (USA)"]
        );
    }
}
