//! Staging attachments for the next message: images grabbed from the system
//! clipboard (Ctrl+V) and files named with `/attach <path>`.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

/// One attachment waiting in the compose box to be uploaded with the next
/// message.
#[derive(Debug, Clone)]
pub struct StagedAttachment {
    pub filename: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

impl StagedAttachment {
    pub fn size_label(&self) -> String {
        human_size(self.bytes.len())
    }
}

pub fn human_size(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

/// Image MIME types we know how to name, most preferred first.
const IMAGE_TYPES: &[(&str, &str)] = &[
    ("image/png", "png"),
    ("image/jpeg", "jpg"),
    ("image/webp", "webp"),
    ("image/gif", "gif"),
    ("image/bmp", "bmp"),
    ("image/tiff", "tiff"),
    ("image/avif", "avif"),
];

pub fn content_type_for_extension(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        "txt" | "log" | "md" => "text/plain",
        "json" => "application/json",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

fn extension_for_image_type(mime: &str) -> &'static str {
    IMAGE_TYPES
        .iter()
        .find(|(m, _)| *m == mime)
        .map(|(_, ext)| *ext)
        .unwrap_or("bin")
}

fn pick_image_type<'a>(offered: impl Iterator<Item = &'a str>) -> Option<String> {
    let offered: Vec<String> = offered
        .map(|t| t.trim().to_ascii_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    for (mime, _) in IMAGE_TYPES {
        if offered.iter().any(|t| t == mime) {
            return Some((*mime).to_string());
        }
    }
    offered.into_iter().find(|t| t.starts_with("image/"))
}

fn clipboard_filename(mime: &str) -> String {
    format!(
        "clipboard-{}.{}",
        chrono::Local::now().format("%Y%m%d-%H%M%S"),
        extension_for_image_type(mime)
    )
}

fn run(cmd: &str, args: &[&str]) -> Result<std::process::Output> {
    Command::new(cmd)
        .args(args)
        .output()
        .with_context(|| format!("failed to run {cmd}"))
}

/// Read an image from the clipboard. Tries wl-paste (Wayland) first, then
/// xclip (X11). Text on the clipboard is not an attachment, so that fails
/// with a message naming what the clipboard actually holds.
fn read_clipboard_image() -> Result<StagedAttachment> {
    let mut last_err: Option<anyhow::Error> = None;

    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        match run("wl-paste", &["--list-types"]) {
            Ok(list) if list.status.success() => {
                let types = String::from_utf8_lossy(&list.stdout);
                let Some(mime) = pick_image_type(types.lines()) else {
                    let seen: Vec<&str> = types.lines().take(4).collect();
                    bail!(
                        "no image on the clipboard (it holds: {})",
                        if seen.is_empty() {
                            "nothing".to_string()
                        } else {
                            seen.join(", ")
                        }
                    );
                };
                let out = run("wl-paste", &["--no-newline", "--type", &mime])?;
                if !out.status.success() || out.stdout.is_empty() {
                    bail!("wl-paste returned no data for {mime}");
                }
                return Ok(StagedAttachment {
                    filename: clipboard_filename(&mime),
                    content_type: mime,
                    bytes: out.stdout,
                });
            }
            Ok(list) => {
                let msg = String::from_utf8_lossy(&list.stderr).trim().to_string();
                last_err = Some(anyhow::anyhow!("wl-paste: {msg}"));
            }
            Err(e) => last_err = Some(e),
        }
    }

    match run("xclip", &["-selection", "clipboard", "-t", "TARGETS", "-o"]) {
        Ok(list) if list.status.success() => {
            let types = String::from_utf8_lossy(&list.stdout);
            let Some(mime) = pick_image_type(types.lines()) else {
                bail!("no image on the clipboard");
            };
            let out = run("xclip", &["-selection", "clipboard", "-t", &mime, "-o"])?;
            if !out.status.success() || out.stdout.is_empty() {
                bail!("xclip returned no data for {mime}");
            }
            return Ok(StagedAttachment {
                filename: clipboard_filename(&mime),
                content_type: mime,
                bytes: out.stdout,
            });
        }
        Ok(list) => {
            let msg = String::from_utf8_lossy(&list.stderr).trim().to_string();
            last_err = Some(anyhow::anyhow!("xclip: {msg}"));
        }
        Err(e) => {
            if last_err.is_none() {
                last_err = Some(e);
            }
        }
    }

    Err(last_err.unwrap_or_else(|| {
        anyhow::anyhow!("no clipboard tool found (install wl-clipboard or xclip)")
    }))
}

pub async fn from_clipboard() -> Result<StagedAttachment> {
    tokio::task::spawn_blocking(read_clipboard_image)
        .await
        .context("clipboard task")?
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(path)
}

pub async fn from_path(raw: &str) -> Result<StagedAttachment> {
    let path = expand_home(raw.trim().trim_matches('"').trim_matches('\''));
    let bytes = tokio::fs::read(&path)
        .await
        .with_context(|| format!("cannot read {}", path.display()))?;
    if bytes.is_empty() {
        bail!("{} is empty", path.display());
    }
    let filename = Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "file".to_string());
    let ext = Path::new(&filename)
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok(StagedAttachment {
        filename,
        content_type: content_type_for_extension(&ext).to_string(),
        bytes,
    })
}
