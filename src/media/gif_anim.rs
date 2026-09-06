use image::codecs::gif::GifDecoder;
use image::codecs::webp::WebPDecoder;
use image::imageops::FilterType;
use image::{AnimationDecoder, Delay, DynamicImage, Frames};
use std::io::Cursor;
use std::time::Duration;

pub fn is_gif_bytes(bytes: &[u8]) -> bool {
    bytes.len() >= 6 && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"))
}

pub fn is_webp_bytes(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP"
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

/// How much of an animation is kept in memory. Frames are scaled down as
/// they are decoded, so a large source never costs more than its shrunken
/// frames.
#[derive(Debug, Clone, Copy)]
struct Limits {
    /// Frames beyond this are dropped; the loop gets shorter.
    max_frames: usize,
    /// Frames wider or taller than this are scaled down to fit it.
    max_dim: u32,
    /// All frames together may hold this many pixels (four bytes each);
    /// past it every frame is scaled down by the same factor.
    max_total_pixels: u64,
    filter: FilterType,
}

/// Custom emoji cover one or two cells, so small frames are plenty.
const EMOJI_LIMITS: Limits = Limits {
    max_frames: usize::MAX,
    max_dim: 256,
    max_total_pixels: u64::MAX,
    filter: FilterType::Nearest,
};

/// The image preview fills most of the screen, so keep more detail, but
/// cap the whole animation at 64 MiB of pixels.
const PREVIEW_LIMITS: Limits = Limits {
    max_frames: 200,
    max_dim: 512,
    max_total_pixels: 16 * 1024 * 1024,
    filter: FilterType::Triangle,
};

/// Frames and delays of an animated custom emoji (GIF or animated WebP), at
/// most `max_frames` of them; None for still images and anything else.
pub fn decode_animation(
    bytes: &[u8],
    max_frames: usize,
) -> Option<(Vec<DynamicImage>, Vec<Duration>)> {
    decode_with(
        bytes,
        Limits {
            max_frames,
            ..EMOJI_LIMITS
        },
    )
}

/// Frames and delays of an animated GIF or animated WebP for the image
/// preview overlay; None for still images and anything else.
pub fn decode_preview_animation(bytes: &[u8]) -> Option<(Vec<DynamicImage>, Vec<Duration>)> {
    decode_with(bytes, PREVIEW_LIMITS)
}

fn animation_frames(bytes: &[u8]) -> Option<Frames<'_>> {
    if is_gif_bytes(bytes) {
        Some(GifDecoder::new(Cursor::new(bytes)).ok()?.into_frames())
    } else if is_webp_bytes(bytes) {
        let decoder = WebPDecoder::new(Cursor::new(bytes)).ok()?;
        if !decoder.has_animation() {
            return None;
        }
        Some(decoder.into_frames())
    } else {
        None
    }
}

fn decode_with(bytes: &[u8], limits: Limits) -> Option<(Vec<DynamicImage>, Vec<Duration>)> {
    let mut frames = Vec::new();
    let mut delays = Vec::new();
    for frame in animation_frames(bytes)?.take(limits.max_frames) {
        // A frame that fails to decode ends the animation there; what
        // played until then still plays, as it does in a browser.
        let Ok(frame) = frame else {
            break;
        };
        let delay = delay_to_duration(frame.delay());
        let img = DynamicImage::ImageRgba8(frame.into_buffer());
        frames.push(shrink_to(img, limits.max_dim, limits.filter));
        delays.push(delay);
    }
    if frames.len() <= 1 {
        return None;
    }
    fit_pixel_budget(&mut frames, limits.max_total_pixels, limits.filter);
    Some((frames, delays))
}

fn shrink_to(img: DynamicImage, max_dim: u32, filter: FilterType) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    if w <= max_dim && h <= max_dim {
        return img;
    }
    let scale = (max_dim as f64 / w as f64).min(max_dim as f64 / h as f64);
    scale_frame(img, scale, filter)
}

fn fit_pixel_budget(frames: &mut [DynamicImage], max_total_pixels: u64, filter: FilterType) {
    let total: u64 = frames
        .iter()
        .map(|f| f.width() as u64 * f.height() as u64)
        .sum();
    if total <= max_total_pixels {
        return;
    }
    let scale = (max_total_pixels as f64 / total as f64).sqrt();
    for frame in frames.iter_mut() {
        let img = std::mem::replace(frame, DynamicImage::new_rgba8(1, 1));
        *frame = scale_frame(img, scale, filter);
    }
}

fn scale_frame(img: DynamicImage, scale: f64, filter: FilterType) -> DynamicImage {
    let nw = ((img.width() as f64 * scale).round() as u32).max(1);
    let nh = ((img.height() as f64 * scale).round() as u32).max(1);
    img.resize_exact(nw, nh, filter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Frame, Rgba, RgbaImage};

    fn animated_gif(frames: usize, w: u32, h: u32, delay_ms: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut out);
            enc.set_repeat(Repeat::Infinite).unwrap();
            for i in 0..frames {
                let px = Rgba([(i * 60) as u8, 0, 255 - (i * 60) as u8, 255]);
                let img = RgbaImage::from_pixel(w, h, px);
                let delay = Delay::from_numer_denom_ms(delay_ms, 1);
                enc.encode_frame(Frame::from_parts(img, 0, 0, delay))
                    .unwrap();
            }
        }
        out
    }

    #[test]
    fn preview_decodes_every_frame_with_its_delay() {
        let bytes = animated_gif(3, 8, 6, 80);
        let (frames, delays) = decode_preview_animation(&bytes).expect("animated");
        assert_eq!(frames.len(), 3);
        assert_eq!(delays, vec![Duration::from_millis(80); 3]);
        assert_eq!((frames[0].width(), frames[0].height()), (8, 6));
    }

    #[test]
    fn single_frame_gif_is_not_an_animation() {
        let bytes = animated_gif(1, 4, 4, 100);
        assert!(decode_preview_animation(&bytes).is_none());
        assert!(decode_animation(&bytes, 48).is_none());
    }

    #[test]
    fn other_formats_are_not_animations() {
        let mut png = Vec::new();
        RgbaImage::from_pixel(2, 2, Rgba([1, 2, 3, 255]))
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        assert!(decode_preview_animation(&png).is_none());
        assert!(decode_preview_animation(b"not an image at all").is_none());
    }

    #[test]
    fn frame_cap_shortens_the_loop() {
        let bytes = animated_gif(5, 4, 4, 100);
        let (frames, _) = decode_animation(&bytes, 2).expect("animated");
        assert_eq!(frames.len(), 2);
    }

    #[test]
    fn zero_delay_plays_at_browser_speed() {
        let bytes = animated_gif(2, 4, 4, 0);
        let (_, delays) = decode_preview_animation(&bytes).expect("animated");
        assert_eq!(delays, vec![Duration::from_millis(100); 2]);
    }

    #[test]
    fn oversized_frames_shrink_to_the_dimension_cap() {
        let img = DynamicImage::new_rgba8(1200, 600);
        let small = shrink_to(img, 512, FilterType::Nearest);
        assert_eq!((small.width(), small.height()), (512, 256));
        let img = DynamicImage::new_rgba8(300, 200);
        let same = shrink_to(img, 512, FilterType::Nearest);
        assert_eq!((same.width(), same.height()), (300, 200));
    }

    #[test]
    fn pixel_budget_scales_all_frames_alike() {
        let mut frames = vec![DynamicImage::new_rgba8(400, 200); 4];
        // 4 × 80 000 = 320 000 pixels into a budget of 80 000: halve each side.
        fit_pixel_budget(&mut frames, 80_000, FilterType::Nearest);
        for f in &frames {
            assert_eq!((f.width(), f.height()), (200, 100));
        }
        let mut frames = vec![DynamicImage::new_rgba8(40, 20); 2];
        fit_pixel_budget(&mut frames, 80_000, FilterType::Nearest);
        assert_eq!((frames[0].width(), frames[0].height()), (40, 20));
    }
}
