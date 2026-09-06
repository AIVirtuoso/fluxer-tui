use image::codecs::gif::GifDecoder;
use image::codecs::webp::WebPDecoder;
use image::{AnimationDecoder, Delay, DynamicImage};
use std::io::Cursor;
use std::time::Duration;

pub fn is_gif_bytes(bytes: &[u8]) -> bool {
    bytes.len() >= 6 && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"))
}

/// Frame delay as browsers (and so the web app) interpret it: a missing
/// delay or one of 10 ms or less plays at 100 ms, anything else is honoured
/// down to 20 ms.
fn delay_to_duration(delay: Delay) -> Duration {
    let (n, d) = delay.numer_denom_ms();
    if d == 0 || n == 0 {
        return Duration::from_millis(100);
    }
    let ms = (n as f64 / d as f64).round();
    if ms <= 10.0 {
        return Duration::from_millis(100);
    }
    Duration::from_millis(ms.clamp(20.0, 30_000.0) as u64)
}

const MAX_FRAME_DIM: u32 = 256;

pub fn is_webp_bytes(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP"
}

/// Frames and delays of an animated GIF or animated WebP, at most
/// `max_frames` of them; None for still images and anything else.
pub fn decode_animation(
    bytes: &[u8],
    max_frames: usize,
) -> Option<(Vec<DynamicImage>, Vec<Duration>)> {
    let raw = if is_gif_bytes(bytes) {
        GifDecoder::new(Cursor::new(bytes))
            .ok()?
            .into_frames()
            .take(max_frames)
            .collect::<Result<Vec<_>, _>>()
            .ok()?
    } else if is_webp_bytes(bytes) {
        let decoder = WebPDecoder::new(Cursor::new(bytes)).ok()?;
        if !decoder.has_animation() {
            return None;
        }
        decoder
            .into_frames()
            .take(max_frames)
            .collect::<Result<Vec<_>, _>>()
            .ok()?
    } else {
        return None;
    };
    frames_from_raw(raw, max_frames)
}

pub fn decode_gif_animation(bytes: &[u8]) -> Option<(Vec<DynamicImage>, Vec<Duration>)> {
    if !is_gif_bytes(bytes) {
        return None;
    }
    let decoder = GifDecoder::new(Cursor::new(bytes)).ok()?;
    let raw = decoder.into_frames().collect_frames().ok()?;
    frames_from_raw(raw, 200)
}

fn frames_from_raw(
    raw: Vec<image::Frame>,
    max_frames: usize,
) -> Option<(Vec<DynamicImage>, Vec<Duration>)> {
    if raw.len() <= 1 {
        return None;
    }

    const MAX_PIXELS: u64 = 1024 * 1024;

    let take = raw.len().min(max_frames);
    let mut frames = Vec::with_capacity(take);
    let mut delays = Vec::with_capacity(take);

    for f in raw.into_iter().take(take) {
        let delay = delay_to_duration(f.delay());
        let buf = f.into_buffer();
        let px = buf.width() as u64 * buf.height() as u64;
        if px > MAX_PIXELS {
            return None;
        }
        let img = DynamicImage::ImageRgba8(buf);
        let img = shrink_frame(img);
        delays.push(delay);
        frames.push(img);
    }

    if frames.len() <= 1 {
        return None;
    }

    Some((frames, delays))
}

fn shrink_frame(img: DynamicImage) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    if w <= MAX_FRAME_DIM && h <= MAX_FRAME_DIM {
        return img;
    }
    let scale = (MAX_FRAME_DIM as f64 / w as f64).min(MAX_FRAME_DIM as f64 / h as f64);
    let nw = ((w as f64 * scale).round() as u32).max(1);
    let nh = ((h as f64 * scale).round() as u32).max(1);
    img.resize_exact(nw, nh, image::imageops::FilterType::Nearest)
}
