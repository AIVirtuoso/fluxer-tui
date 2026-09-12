use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn known_media_extension(name: &str) -> Option<&'static str> {
    let ext = Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .map(|e| e.to_ascii_lowercase())?;
    Some(match ext.as_str() {
        "mp4" => "mp4",
        "webm" => "webm",
        "mov" => "mov",
        "mkv" => "mkv",
        "avi" => "avi",
        "m4v" => "m4v",
        "ogv" => "ogv",
        "gif" => "gif",
        "webp" => "webp",
        _ => return None,
    })
}

/// Last path segment of a URL, without query string or fragment.
fn url_file_name(url: &str) -> &str {
    let end = url.find(['?', '#']).unwrap_or(url.len());
    let path = &url[..end];
    path.rsplit('/').next().unwrap_or(path)
}

/// Container type from the first bytes: Matroska/WebM, ISO BMFF (mp4, m4v,
/// mov), GIF or WebP.
fn sniff_media_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some("webm");
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        return Some("mp4");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("gif");
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("webp");
    }
    None
}

/// Extension for the temp file: from the label (an attachment's file name),
/// else from the URL's file name (GIF providers name their video copies
/// `….webm`), else from the bytes themselves, else mp4.
fn media_extension(label: &str, url: &str, bytes: &[u8]) -> &'static str {
    known_media_extension(label)
        .or_else(|| known_media_extension(url_file_name(url)))
        .or_else(|| sniff_media_extension(bytes))
        .unwrap_or("mp4")
}

/// Write bytes to a unique file under the system temp dir (for opening with an external app).
pub fn write_temp_video_bytes(label: &str, url: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    let ext = media_extension(label, url, bytes);
    let uniq = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = format!("fluxer-tui-video-{uniq}.{ext}");
    let path = std::env::temp_dir().join(name);
    fs::write(&path, bytes)?;
    Ok(path)
}

fn command_ok(name: &str, st: std::process::ExitStatus) -> io::Result<()> {
    if st.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("{name} exited with {st}")))
    }
}

/// Open a file with the desktop default application (video player, etc.).
pub fn open_file_path(path: &Path) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let st = Command::new("open").arg(path).status()?;
        return command_ok("open", st);
    }
    #[cfg(target_os = "windows")]
    {
        let s = path.as_os_str().to_string_lossy().into_owned();
        let st = Command::new("cmd").args(["/C", "start", "", &s]).status()?;
        return command_ok("cmd /C start", st);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let st = Command::new("xdg-open").arg(path).status()?;
        command_ok("xdg-open", st)
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "windows",
        all(unix, not(target_os = "macos"))
    )))]
    {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "no open command for this OS",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_prefers_label_then_url_then_bytes() {
        assert_eq!(media_extension("clip.MOV", "https://x/y.webm", b""), "mov");
        assert_eq!(
            media_extension(
                "linux-kernel-tux",
                "https://cdn.example/a/b/DkIvrEVx48Lh.webm?sig=1#f",
                b""
            ),
            "webm"
        );
        assert_eq!(
            media_extension(
                "video",
                "https://klipy.com/gifs/x",
                &[0x1A, 0x45, 0xDF, 0xA3, 0, 0]
            ),
            "webm"
        );
        assert_eq!(
            media_extension("video", "https://x/", b"\0\0\0\x18ftypisom\0\0\0\0"),
            "mp4"
        );
        assert_eq!(media_extension("video", "https://x/page", b"<html>"), "mp4");
    }

    #[test]
    fn url_file_name_strips_query_and_fragment() {
        assert_eq!(url_file_name("https://h/p/a.webm?x=1#y"), "a.webm");
        assert_eq!(url_file_name("https://h/p/"), "");
        assert_eq!(url_file_name("plain"), "plain");
    }
}
