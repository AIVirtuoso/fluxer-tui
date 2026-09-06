//! Locating fonts for console mode. Explicit paths from the config win;
//! otherwise fontconfig's `fc-match` answers for "monospace" and "emoji".

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Default)]
pub struct FontPaths {
    pub text: Option<PathBuf>,
    pub bold: Option<PathBuf>,
    pub emoji: Option<PathBuf>,
}

fn fc_match(pattern: &str) -> Option<PathBuf> {
    let out = Command::new("fc-match")
        .args(["-f", "%{file}", pattern])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if path.is_empty() {
        return None;
    }
    let p = PathBuf::from(path);
    p.is_file().then_some(p)
}

fn readable(p: &Path) -> bool {
    std::fs::metadata(p).map(|m| m.is_file()).unwrap_or(false)
}

/// Resolve the fonts to use. `text` is required; bold and emoji are optional
/// (bold falls back to synthetic emboldening, emoji to the text font).
pub fn resolve(requested: &FontPaths) -> Result<FontPaths> {
    let text = match &requested.text {
        Some(p) if readable(p) => p.clone(),
        Some(p) => bail!("console font not found: {}", p.display()),
        None => fc_match("monospace")
            .context("no monospace font found; set [console] font in the config")?,
    };
    let bold = match &requested.bold {
        Some(p) if readable(p) => Some(p.clone()),
        Some(p) => bail!("console bold font not found: {}", p.display()),
        None => fc_match("monospace:bold").filter(|p| *p != text),
    };
    let emoji = match &requested.emoji {
        Some(p) if readable(p) => Some(p.clone()),
        Some(p) => bail!("console emoji font not found: {}", p.display()),
        None => fc_match("emoji").filter(|p| *p != text),
    };
    Ok(FontPaths {
        text: Some(text),
        bold,
        emoji,
    })
}
