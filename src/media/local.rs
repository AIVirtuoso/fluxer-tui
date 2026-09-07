//! Pictures of what is about to be sent: a file on disk (the file picker)
//! or bytes staged in the compose box. An image is shown as it is; a
//! video's first frame comes from `ffmpeg` on PATH, letterboxed into the
//! block, and there is no preview without it. Both go through the same
//! media pipeline as pictures in chat, under their own URL schemes.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const FILE_SCHEME: &str = "file://";
pub const STAGED_SCHEME: &str = "staged://";

const IMAGE_EXT: &[&str] = &[
    ".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".avif", ".tif", ".tiff",
];
const VIDEO_EXT: &[&str] = &[".mp4", ".webm", ".mkv", ".mov", ".avi", ".m4v", ".ogv"];

pub fn is_image_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    IMAGE_EXT.iter().any(|e| n.ends_with(e))
}

pub fn is_video_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    VIDEO_EXT.iter().any(|e| n.ends_with(e))
}

/// An image the `image` crate can decode (SVG is not one).
pub fn is_image(content_type: &str, name: &str) -> bool {
    (content_type.starts_with("image/") && content_type != "image/svg+xml") || is_image_name(name)
}

pub fn is_video(content_type: &str, name: &str) -> bool {
    content_type.starts_with("video/") || is_video_name(name)
}

/// Where a local picture's bytes come from.
#[derive(Debug, Clone)]
pub enum LocalSource {
    Path(PathBuf),
    Bytes { filename: String, bytes: Vec<u8> },
}

pub fn file_url(path: &Path) -> String {
    format!("{FILE_SCHEME}{}", path.display())
}

pub fn staged_url(id: u64) -> String {
    format!("{STAGED_SCHEME}{id}")
}

pub fn parse_file_url(url: &str) -> Option<PathBuf> {
    url.strip_prefix(FILE_SCHEME).map(PathBuf::from)
}

pub fn parse_staged_url(url: &str) -> Option<u64> {
    url.strip_prefix(STAGED_SCHEME)?.parse().ok()
}

/// The bytes of a picture for a block `box_px` pixels big: an image file
/// as it is, a video's first frame letterboxed into the box, None when
/// there is nothing to show (or no ffmpeg).
pub fn picture_bytes(source: LocalSource, box_px: (u32, u32)) -> Option<Vec<u8>> {
    match source {
        LocalSource::Path(path) => {
            let name = path.file_name()?.to_string_lossy().to_string();
            if is_video_name(&name) {
                video_poster(&path, box_px)
            } else if is_image_name(&name) {
                std::fs::read(&path).ok()
            } else {
                None
            }
        }
        LocalSource::Bytes { filename, bytes } => {
            if is_video_name(&filename) {
                // ffmpeg wants a file it can seek in
                let ext = Path::new(&filename)
                    .extension()
                    .map(|e| format!(".{}", e.to_string_lossy()))
                    .unwrap_or_default();
                let mut tmp = tempfile::Builder::new()
                    .prefix("fluxer-tui-staged-")
                    .suffix(&ext)
                    .tempfile()
                    .ok()?;
                std::io::Write::write_all(&mut tmp, &bytes).ok()?;
                video_poster(tmp.path(), box_px)
            } else if is_image_name(&filename) || image_dimensions(&bytes).is_some() {
                Some(bytes)
            } else {
                None
            }
        }
    }
}

/// The arguments that make ffmpeg print a video's first frame as a PNG
/// that fills `box_px`, the picture centred with bars where its shape
/// differs.
fn poster_args(path: &Path, (w, h): (u32, u32)) -> Vec<String> {
    let (w, h) = (w.max(2), h.max(2));
    let filter = format!(
        "scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2"
    );
    [
        "-v",
        "error",
        "-nostdin",
        "-y",
        "-i",
        &path.to_string_lossy(),
        "-frames:v",
        "1",
        "-vf",
        &filter,
        "-f",
        "image2",
        "-c:v",
        "png",
        "-",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn video_poster(path: &Path, box_px: (u32, u32)) -> Option<Vec<u8>> {
    let out = Command::new("ffmpeg")
        .args(poster_args(path, box_px))
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    (out.status.success() && !out.stdout.is_empty()).then_some(out.stdout)
}

/// Pixel size of an image from its header alone.
pub fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

pub fn image_dimensions_of(path: &Path) -> Option<(u32, u32)> {
    image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

/// The files named by a `text/uri-list` (what a file manager puts on the
/// clipboard when files are copied): `file:` URIs, one per line, comments
/// starting with `#`, percent-encoded.
pub fn uri_list_paths(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let rest = l.strip_prefix("file:")?;
            // file:///path, or file://host/path
            let path = match rest.strip_prefix("//") {
                Some(after) if after.starts_with('/') => after,
                Some(after) => &after[after.find('/')?..],
                None => rest,
            };
            let decoded = urlencoding::decode(path).ok()?.into_owned();
            (!decoded.is_empty()).then(|| PathBuf::from(decoded))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_tell_images_and_videos_apart() {
        assert!(is_image_name("Cat.PNG") && !is_video_name("Cat.PNG"));
        assert!(is_video_name("clip.mkv") && !is_image_name("clip.mkv"));
        assert!(is_image("image/png", "x") && !is_image("image/svg+xml", "logo.svg"));
        assert!(is_video("video/mp4", "x") && !is_video("audio/mpeg", "song.mp3"));
    }

    #[test]
    fn slot_urls_round_trip() {
        let p = Path::new("/home/me/pic one.png");
        assert_eq!(parse_file_url(&file_url(p)).as_deref(), Some(p));
        assert_eq!(parse_staged_url(&staged_url(42)), Some(42));
        assert_eq!(parse_staged_url("https://x/y"), None);
        assert_eq!(parse_file_url("staged://1"), None);
    }

    #[test]
    fn uri_lists_become_paths() {
        let paths = uri_list_paths(
            "# copied\r\nfile:///home/me/a%20b.png\r\nfile://localhost/tmp/c.mp4\nhttps://x/y\n\n",
        );
        assert_eq!(
            paths,
            [
                PathBuf::from("/home/me/a b.png"),
                PathBuf::from("/tmp/c.mp4")
            ]
        );
    }

    #[test]
    fn image_bytes_pass_through_and_others_do_not() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(3, 2, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        assert_eq!(image_dimensions(&png), Some((3, 2)));
        let got = picture_bytes(
            LocalSource::Bytes {
                filename: "shot".into(),
                bytes: png.clone(),
            },
            (30, 20),
        );
        assert_eq!(got, Some(png));
        assert!(
            picture_bytes(
                LocalSource::Bytes {
                    filename: "notes.txt".into(),
                    bytes: b"hello".to_vec(),
                },
                (30, 20),
            )
            .is_none()
        );
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("readme.md");
        std::fs::write(&p, "# hi").unwrap();
        assert!(picture_bytes(LocalSource::Path(p), (30, 20)).is_none());
    }

    /// With ffmpeg on PATH: a video's first frame fills the box.
    #[test]
    fn a_videos_first_frame_fills_the_box() {
        if Command::new("ffmpeg").arg("-version").output().is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let clip = dir.path().join("clip.mp4");
        let made = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=64x48:rate=5",
            ])
            .args(["-t", "0.4", "-pix_fmt", "yuv420p"])
            .arg(&clip)
            .output()
            .unwrap();
        assert!(made.status.success(), "ffmpeg could not make a clip");
        let png = picture_bytes(LocalSource::Path(clip.clone()), (160, 80)).expect("a poster");
        assert_eq!(
            image_dimensions(&png),
            Some((160, 80)),
            "letterboxed into the box"
        );
        let bytes = std::fs::read(&clip).unwrap();
        let png = picture_bytes(
            LocalSource::Bytes {
                filename: "clip.mp4".into(),
                bytes,
            },
            (32, 32),
        )
        .expect("a poster from bytes");
        assert_eq!(image_dimensions(&png), Some((32, 32)));
    }

    #[test]
    fn poster_arguments_letterbox_into_the_box() {
        let args = poster_args(Path::new("/v/clip.mp4"), (160, 90));
        let joined = args.join(" ");
        assert!(joined.starts_with("-v error -nostdin -y -i /v/clip.mp4 -frames:v 1 -vf "));
        assert!(joined.contains("scale=160:90:force_original_aspect_ratio=decrease,pad=160:90:"));
        assert!(joined.ends_with("-f image2 -c:v png -"));
    }
}
