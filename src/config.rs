//! Loading `config.toml`: the ordered list of VIA layout files to swap between.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    layouts: Vec<RawEntry>,
}

/// A layout entry is either a bare path string, or a table with a path and an optional name.
#[derive(Deserialize)]
#[serde(untagged)]
enum RawEntry {
    Path(String),
    Full { path: String, name: Option<String> },
}

/// One configured layout: a display name and the resolved path to its VIA JSON export.
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
}

fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .context("$HOME is not set")
}

/// `$XDG_CONFIG_HOME/keyboard-swap/config.toml`, falling back to `~/.config/...`.
pub fn default_path() -> Result<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => home()?.join(".config"),
    };
    Ok(base.join("keyboard-swap").join("config.toml"))
}

/// Expands a leading `~`, and resolves relative paths against the config file's directory.
fn resolve(raw: &str, base: &Path) -> Result<PathBuf> {
    let path = if raw == "~" {
        home()?
    } else if let Some(rest) = raw.strip_prefix("~/") {
        home()?.join(rest)
    } else {
        PathBuf::from(raw)
    };
    Ok(if path.is_relative() { base.join(path) } else { path })
}

pub const EXAMPLE: &str = r#"# Layouts are listed in cycling order. Each is a VIA JSON export.
# A bare string uses the file name (without .json) as the layout's name.
layouts = [
    "~/path/to/standard.json",
    { path = "~/path/to/gamer.json", name = "gaming" },
]
"#;

pub fn load(path: &Path) -> Result<Vec<Entry>> {
    let text = std::fs::read_to_string(path).with_context(|| {
        format!(
            "could not read config file {}\n\nCreate it with contents like:\n\n{EXAMPLE}",
            path.display()
        )
    })?;
    let raw: RawConfig =
        toml::from_str(&text).with_context(|| format!("could not parse {}", path.display()))?;
    if raw.layouts.is_empty() {
        bail!("{}: `layouts` is empty", path.display());
    }

    let base = path.parent().unwrap_or(Path::new("."));
    let mut seen = HashSet::new();
    let mut entries = Vec::with_capacity(raw.layouts.len());
    for raw_entry in raw.layouts {
        let (raw_path, name) = match raw_entry {
            RawEntry::Path(p) => (p, None),
            RawEntry::Full { path, name } => (path, name),
        };
        let path = resolve(&raw_path, base)?;
        let name = match name {
            Some(n) => n,
            None => path
                .file_stem()
                .and_then(|s| s.to_str())
                .with_context(|| format!("can't derive a name from {raw_path:?}; give it a `name`"))?
                .to_owned(),
        };
        if !seen.insert(name.clone()) {
            bail!("{}: two layouts are both named {name:?}", path.display());
        }
        entries.push(Entry { name, path });
    }
    Ok(entries)
}
