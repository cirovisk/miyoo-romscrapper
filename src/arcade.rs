use anyhow::{Context, Result};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};
use xmltree::{Element, XMLNode};

const ALIASES_FILE: &str = ".rom-art-arcade-aliases.json";

/// Resolve an arcade short name using a nearby DAT/XML catalog, existing game lists, then saved aliases.
pub fn title(root: &Path, rom: &Path) -> Result<Option<String>> {
    let short_name = rom.file_stem().and_then(|v| v.to_str()).unwrap_or_default();
    if let Some(name) = catalog_title(root, short_name) {
        return Ok(Some(name));
    }
    if let Some(name) = gamelist_title(root, rom)? {
        return Ok(Some(name));
    }
    let aliases = read_aliases(root)?;
    Ok(aliases.get(&relative_key(root, rom)?).cloned())
}

pub fn remember(root: &Path, rom: &Path, name: &str) -> Result<()> {
    let path = root.join(ALIASES_FILE);
    let mut aliases = read_aliases(root)?;
    aliases.insert(relative_key(root, rom)?, name.to_owned());
    let bytes = serde_json::to_vec_pretty(&aliases)?;
    crate::storage::save_atomic(&path, &bytes, true)
}

fn relative_key(root: &Path, rom: &Path) -> Result<String> {
    Ok(rom
        .strip_prefix(root)
        .context("Arcade ROM is outside its system folder")?
        .to_string_lossy()
        .replace('\\', "/"))
}

fn read_aliases(root: &Path) -> Result<HashMap<String, String>> {
    let path = root.join(ALIASES_FILE);
    match fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes).context("Invalid arcade alias file")?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
        Err(error) => Err(error.into()),
    }
}

fn gamelist_title(root: &Path, rom: &Path) -> Result<Option<String>> {
    let relative = PathBuf::from(format!("./{}", relative_key(root, rom)?));
    for filename in ["gamelist.xml", "miyoogamelist.xml"] {
        let path = root.join(filename);
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        let document = Element::parse(bytes.as_slice()).context("Invalid existing game list")?;
        for game in document.children.iter().filter_map(as_element) {
            if game.name != "game" {
                continue;
            }
            let game_path = game.get_child("path").and_then(Element::get_text);
            if game_path
                .as_deref()
                .is_some_and(|p| Path::new(p) == relative)
                && let Some(name) = game.get_child("name").and_then(Element::get_text)
            {
                let name = name.trim();
                if !name.is_empty() {
                    return Ok(Some(name.to_owned()));
                }
            }
        }
    }
    Ok(None)
}

fn as_element(node: &XMLNode) -> Option<&Element> {
    match node {
        XMLNode::Element(element) => Some(element),
        _ => None,
    }
}

fn catalog_title(root: &Path, short_name: &str) -> Option<String> {
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file()
            || !path.extension().is_some_and(|ext| {
                matches!(
                    ext.to_string_lossy().to_ascii_lowercase().as_str(),
                    "dat" | "xml"
                )
            })
        {
            continue;
        }
        if let Ok(bytes) = fs::read(&path)
            && let Some(name) = parse_catalog(&bytes, short_name)
        {
            return Some(name);
        }
    }
    None
}

fn parse_catalog(bytes: &[u8], short_name: &str) -> Option<String> {
    if let Ok(document) = Element::parse(bytes) {
        let mut found = None;
        walk_xml(&document, short_name, &mut found);
        if found.is_some() {
            return found;
        }
    }
    parse_dat(bytes, short_name)
}

fn walk_xml(element: &Element, short_name: &str, found: &mut Option<String>) {
    if found.is_some() {
        return;
    }
    if matches!(element.name.as_str(), "machine" | "game")
        && element
            .attributes
            .get("name")
            .is_some_and(|name| name == short_name)
        && let Some(description) = element.attributes.get("description").cloned().or_else(|| {
            element
                .get_child("description")
                .and_then(Element::get_text)
                .map(|v| v.into_owned())
        })
    {
        let description = description.trim();
        if !description.is_empty() {
            *found = Some(description.to_owned());
            return;
        }
    }
    for child in element.children.iter().filter_map(as_element) {
        walk_xml(child, short_name, found);
    }
}

fn parse_dat(bytes: &[u8], short_name: &str) -> Option<String> {
    let source = std::str::from_utf8(bytes).ok()?;
    let tokens = dat_tokens(source);
    let mut stack: Vec<(String, Option<String>, Option<String>)> = Vec::new();
    let mut pending = None;
    let mut index = 0;
    while index < tokens.len() {
        match tokens[index].as_str() {
            "(" => {
                if let Some(kind) = tokens.get(index + 1) {
                    stack.push((kind.clone(), None, None));
                    index += 1;
                }
            }
            "game" | "machine" if tokens.get(index + 1).is_some_and(|v| v == "(") => {
                stack.push((tokens[index].clone(), None, None));
                index += 1;
            }
            ")" => {
                if let Some((kind, name, description)) = stack.pop()
                    && matches!(kind.as_str(), "game" | "machine")
                    && name.as_deref() == Some(short_name)
                    && let Some(description) = description.filter(|v| !v.trim().is_empty())
                {
                    return Some(description.trim().to_owned());
                }
            }
            key if key == "name" || key == "description" => {
                pending = Some(key.to_owned());
            }
            value => {
                if let (Some(key), Some(frame)) = (pending.take(), stack.last_mut()) {
                    match key.as_str() {
                        "name" => frame.1 = Some(value.to_owned()),
                        "description" => frame.2 = Some(value.to_owned()),
                        _ => {}
                    }
                }
            }
        }
        index += 1;
    }
    None
}

fn dat_tokens(source: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '(' | ')' => tokens.push(ch.to_string()),
            '"' => {
                let mut value = String::new();
                while let Some(ch) = chars.next() {
                    match ch {
                        '"' => break,
                        '\\' => {
                            if let Some(escaped) = chars.next() {
                                value.push(escaped);
                            }
                        }
                        _ => value.push(ch),
                    }
                }
                tokens.push(value);
            }
            ch if ch.is_whitespace() => {}
            _ => {
                let mut value = ch.to_string();
                while let Some(ch) = chars.peek() {
                    if ch.is_whitespace() || matches!(ch, '(' | ')') {
                        break;
                    }
                    value.push(chars.next().unwrap());
                }
                tokens.push(value);
            }
        }
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mame_xml_and_fbneo_dat() {
        assert_eq!(
            parse_catalog(
                br#"<mame><machine name="mslug" description="Metal Slug"/></mame>"#,
                "mslug"
            ),
            Some("Metal Slug".into())
        );
        assert_eq!(
            parse_catalog(
                b"game ( name \"kof98\" description \"The King of Fighters '98\" )",
                "kof98"
            ),
            Some("The King of Fighters '98".into())
        );
    }

    #[test]
    fn aliases_are_saved_per_relative_rom_path() {
        let root = tempfile::tempdir().unwrap();
        let rom = root.path().join("mslug.zip");
        fs::write(&rom, b"rom").unwrap();
        remember(root.path(), &rom, "Metal Slug").unwrap();
        assert_eq!(title(root.path(), &rom).unwrap(), Some("Metal Slug".into()));
    }

    #[test]
    fn reads_existing_gamelist_name_when_catalog_is_missing() {
        let root = tempfile::tempdir().unwrap();
        let rom = root.path().join("mslug.zip");
        fs::write(&rom, b"rom").unwrap();
        fs::write(
            root.path().join("gamelist.xml"),
            b"<gameList><game><path>./mslug.zip</path><name>Metal Slug</name></game></gameList>",
        )
        .unwrap();
        assert_eq!(title(root.path(), &rom).unwrap(), Some("Metal Slug".into()));
    }
}
