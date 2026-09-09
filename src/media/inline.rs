//! Pictures in the message pane: which media a message shows, the cells a
//! preview takes, the proxy URL that delivers it at that size, and the
//! avatar rules of the web app.

use crate::api::types::{MessageAttachmentResponse, MessageEmbedResponse, UserPartialResponse};
use image::imageops::FilterType;
use image::{DynamicImage, Rgba, RgbaImage};
use std::time::Duration;

/// An avatar beside a message: four cells wide, two rows tall, about square.
pub const AVATAR_COLS: u16 = 4;
pub const AVATAR_ROWS: u16 = 2;
/// Avatar size asked of the media proxy: the web app's default, so the
/// proxy already has it.
pub const AVATAR_PX: u32 = 160;
/// A preview under a message is at most this many pixels wide and tall...
pub const PREVIEW_MAX_PX: (u32, u32) = (420, 300);
/// ...and at most a third of the pane, within these rows.
pub const PREVIEW_MAX_ROWS: u16 = 12;
pub const PREVIEW_MIN_ROWS: u16 = 3;
pub const PREVIEW_MAX_COLS: u16 = 64;
/// Pictures shown under one message.
pub const MAX_PICTURES_PER_MESSAGE: usize = 4;

/// Rows a block of cells may have. The marker cells carry their row within
/// the block in four bits (`app::media_marker_style`), so a taller block
/// would repeat row 15 and the overlay, which wants the rows to follow one
/// another, would draw nothing at all.
pub const BLOCK_MAX_ROWS: u16 = 16;
/// Frames kept for an animated preview; longer loops are thinned out.
pub const INLINE_MAX_FRAMES: usize = 48;

/// A picture a message can show: the media-proxy URL of the original and
/// its pixel size, known before anything is downloaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlinePicture {
    pub proxy_url: String,
    pub width: u32,
    pub height: u32,
    pub animated: bool,
}

fn media_proxy_url(url: Option<&str>) -> Option<String> {
    url.filter(|u| u.starts_with("http://") || u.starts_with("https://"))
        .map(str::to_string)
}

/// The preview of an attachment: pictures, and videos as their poster frame.
pub fn attachment_picture(a: &MessageAttachmentResponse) -> Option<InlinePicture> {
    let mime = a.content_type.as_deref().unwrap_or("");
    let name = a.filename.to_lowercase();
    let by_name = [
        ".png", ".jpg", ".jpeg", ".gif", ".webp", ".avif", ".mp4", ".webm", ".mov",
    ]
    .iter()
    .any(|e| name.ends_with(e));
    if !(mime.starts_with("image/") || mime.starts_with("video/") || by_name) {
        return None;
    }
    let (width, height) = (a.width?, a.height?);
    if width == 0 || height == 0 {
        return None;
    }
    let animated = mime == "image/gif"
        || mime == "image/webp"
        || name.ends_with(".gif")
        || name.ends_with(".webp");
    Some(InlinePicture {
        proxy_url: media_proxy_url(a.proxy_url.as_deref())?,
        width,
        height,
        animated,
    })
}

/// The preview of an embed: a GIF provider's animation, a linked image, a
/// video's poster, or a rich embed's picture.
pub fn embed_picture(e: &MessageEmbedResponse) -> Option<InlinePicture> {
    let (media, animated) = match e.embed_type.as_str() {
        "gifv" => (e.thumbnail.as_ref().or(e.image.as_ref())?, true),
        "image" => {
            let m = e.image.as_ref().or(e.thumbnail.as_ref())?;
            (m, m.is_animated())
        }
        "video" => (e.thumbnail.as_ref()?, false),
        _ => {
            let m = e.image.as_ref()?;
            (m, m.is_animated())
        }
    };
    let (width, height) = (media.width?, media.height?);
    if width == 0 || height == 0 {
        return None;
    }
    Some(InlinePicture {
        proxy_url: media_proxy_url(media.proxy_url.as_deref())?,
        width,
        height,
        animated,
    })
}

fn cell_or_default(cell: (u32, u32)) -> (u32, u32) {
    (
        if cell.0 == 0 { 8 } else { cell.0 },
        if cell.1 == 0 { 16 } else { cell.1 },
    )
}

/// Pixel size of a block of cells.
pub fn block_px(cols: u16, rows: u16, cell: (u32, u32)) -> (u32, u32) {
    let (cw, ch) = cell_or_default(cell);
    (cols as u32 * cw, rows as u32 * ch)
}

/// (columns, rows) a preview may take in a pane `text_w` cells wide and
/// `pane_rows` tall, given the pixel size of a cell.
pub fn preview_limits(text_w: u16, pane_rows: u16, cell: (u32, u32)) -> (u16, u16) {
    let (cw, ch) = cell_or_default(cell);
    let cols_px = (PREVIEW_MAX_PX.0 / cw).max(1) as u16;
    let rows_px = (PREVIEW_MAX_PX.1 / ch).max(1) as u16;
    let max_cols = text_w.min(cols_px).clamp(1, PREVIEW_MAX_COLS);
    let max_rows = (pane_rows / 3)
        .clamp(PREVIEW_MIN_ROWS, PREVIEW_MAX_ROWS)
        .min(rows_px)
        .max(1);
    (max_cols, max_rows)
}

/// Pixel size of `img` scaled down (never up) to fit `box_px`.
pub fn fitted_px(img: (u32, u32), box_px: (u32, u32)) -> (u32, u32) {
    if img.0 == 0 || img.1 == 0 {
        return (1, 1);
    }
    let scale = (box_px.0 as f64 / img.0 as f64)
        .min(box_px.1 as f64 / img.1 as f64)
        .min(1.0);
    (
        ((img.0 as f64 * scale).round() as u32).max(1),
        ((img.1 as f64 * scale).round() as u32).max(1),
    )
}

/// Cells (columns, rows) a picture of `img` pixels takes within `max` cells,
/// keeping its shape; a small picture stays small. Rounded to the nearest
/// cell: the picture is then stretched to exactly that many cells, at most
/// half a cell off its true shape. Never taller than [`BLOCK_MAX_ROWS`],
/// so the block is one the overlay can draw.
pub fn picture_cells(img: (u32, u32), cell: (u32, u32), max: (u16, u16)) -> (u16, u16) {
    let (cw, ch) = cell_or_default(cell);
    let max = (max.0, max.1.min(BLOCK_MAX_ROWS));
    let (w, h) = fitted_px(img, block_px(max.0, max.1, cell));
    let cols = ((w as f64 / cw as f64).round() as u32).clamp(1, max.0.max(1) as u32) as u16;
    let rows = ((h as f64 / ch as f64).round() as u32).clamp(1, max.1.max(1) as u32) as u16;
    (cols, rows)
}

/// The proxy URL that delivers the picture already scaled to fit `box_px`,
/// as WebP and animated when it is a GIF: a preview then costs a fraction
/// of the original's download and decoding. Only the limiting side is
/// sent; the proxy keeps the aspect ratio.
pub fn proxied_url(pic: &InlinePicture, box_px: (u32, u32)) -> String {
    let (w, h) = fitted_px((pic.width, pic.height), box_px);
    let mut url = pic.proxy_url.clone();
    let mut push = |key: &str, value: &str| {
        url.push(if url.contains('?') { '&' } else { '?' });
        url.push_str(key);
        url.push('=');
        url.push_str(value);
    };
    if w < pic.width || h < pic.height {
        let width_limits =
            (box_px.0 as u64 * pic.height as u64) <= (box_px.1 as u64 * pic.width as u64);
        if width_limits {
            push("width", &w.to_string());
        } else {
            push("height", &h.to_string());
        }
    }
    push("format", "webp");
    if pic.animated {
        push("animated", "true");
    }
    url
}

/// The media-proxy URL of an avatar, as the web app builds it: an `a_`
/// prefix marks an animated one, which is asked for as a still picture.
pub fn avatar_url(media_base: &str, guild_id: Option<&str>, user_id: &str, hash: &str) -> String {
    avatar_url_sized(media_base, guild_id, user_id, hash, AVATAR_PX)
}

/// The largest size the media proxy serves an avatar at.
pub const AVATAR_PREVIEW_PX: u32 = 1024;

/// An avatar at one of the media proxy's sizes.
pub fn avatar_url_sized(
    media_base: &str,
    guild_id: Option<&str>,
    user_id: &str,
    hash: &str,
    size: u32,
) -> String {
    let base = media_base.trim_end_matches('/');
    let hash = hash.strip_prefix("a_").unwrap_or(hash);
    match guild_id {
        Some(gid) => {
            format!("{base}/guilds/{gid}/users/{user_id}/avatars/{hash}.webp?size={size}")
        }
        None => format!("{base}/avatars/{user_id}/{hash}.webp?size={size}"),
    }
}

/// The web app's default avatar colours, picked by user id.
const DEFAULT_AVATAR_COLORS: [u32; 6] =
    [0x4641d9, 0xf0b100, 0x00bba7, 0x2b7fff, 0xad46ff, 0x6a7282];

/// Colour of a user's default avatar: the one the API sends, else the web
/// app's choice by id.
pub fn default_avatar_color(user: &UserPartialResponse) -> [u8; 3] {
    let packed = user.avatar_color.unwrap_or_else(|| {
        let idx = user
            .id
            .parse::<u128>()
            .map(|id| (id % DEFAULT_AVATAR_COLORS.len() as u128) as usize)
            .unwrap_or(0);
        DEFAULT_AVATAR_COLORS[idx]
    });
    [(packed >> 16) as u8, (packed >> 8) as u8, packed as u8]
}

/// Pseudo URL of an avatar drawn locally: a disc in the user's colour.
pub const DEFAULT_AVATAR_SCHEME: &str = "avatar-color:";

pub fn default_avatar_key(color: [u8; 3]) -> String {
    format!(
        "{DEFAULT_AVATAR_SCHEME}{:02x}{:02x}{:02x}",
        color[0], color[1], color[2]
    )
}

pub fn parse_default_avatar_key(key: &str) -> Option<[u8; 3]> {
    let hex = key.strip_prefix(DEFAULT_AVATAR_SCHEME)?;
    if hex.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(hex, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// A filled disc, for users without a picture.
pub fn disc_image(color: [u8; 3], size: u32) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(size, size, Rgba([color[0], color[1], color[2], 255]));
    circle_mask(&mut img);
    img
}

/// Clear everything outside the inscribed circle, with a soft edge.
pub fn circle_mask(img: &mut RgbaImage) {
    let (w, h) = (img.width() as f32, img.height() as f32);
    let (cx, cy) = (w / 2.0, h / 2.0);
    let r = w.min(h) / 2.0;
    for (x, y, px) in img.enumerate_pixels_mut() {
        let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
        let coverage = (r - d + 0.5).clamp(0.0, 1.0);
        if coverage < 1.0 {
            px.0[3] = (px.0[3] as f32 * coverage).round() as u8;
        }
    }
}

/// Sixel paints six pixel rows per band: a picture whose height is not a
/// multiple of six ends in a partial band that the terminal paints past the
/// picture's cells, a strip that nothing ever erases. Heights are cut down
/// to whole bands; the few rows left at the bottom of the block show the
/// background.
pub fn sixel_rows(height: u32) -> u32 {
    (height / 6 * 6).max(6)
}

/// How a picture's transparency is dealt with on a protocol that carries
/// no alpha channel of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flatten {
    /// The colour transparency is flattened onto.
    pub colour: [u8; 3],
    /// Whether those pixels are then dropped from the sixel, so that the
    /// terminal's own background shows through them exactly rather than
    /// the encoder's nearest guess at it.
    pub drop: bool,
}

/// The sixel encoder buckets colours at five bits a channel, so only
/// multiples of eight survive it: ask for `#002b36` and the picture comes
/// back painted `#002830`. Flattening transparency onto a snapped colour
/// instead means the encoder reproduces it exactly, which both keeps the
/// dithering from spreading an error across the flat area and leaves one
/// palette entry that `sixel_drop_colour` can recognise.
pub fn sixel_snap(rgb: [u8; 3]) -> [u8; 3] {
    rgb.map(|c| c & 0xf8)
}

/// The colour as sixel writes it: components are hundredths, not bytes.
fn sixel_percent(rgb: [u8; 3]) -> [u32; 3] {
    rgb.map(|c| (c as u32 * 100 + 127) / 255)
}

/// Every palette index a sixel defines as `want`. The quantiser can reach
/// the same colour by more than one route and define it twice, and only
/// blanking one of them leaves the other painting.
fn indices_of_colour(data: &str, want: [u32; 3]) -> Vec<u32> {
    let b = data.as_bytes();
    let mut found = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'#' {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        let mut index = 0u32;
        while j < b.len() && b[j].is_ascii_digit() {
            index = index * 10 + u32::from(b[j] - b'0');
            j += 1;
        }
        if j > i + 1 && b.get(j) == Some(&b';') {
            let start = i;
            while j < b.len() && (b[j].is_ascii_digit() || b[j] == b';') {
                j += 1;
            }
            if defined_colour(&data[start..j]) == Some(want) && !found.contains(&index) {
                found.push(index);
            }
        }
        i = j.max(i + 1);
    }
    found
}

/// The colour a `#n;2;r;g;b` definition sets, in sixel's own hundredths.
fn defined_colour(def: &str) -> Option<[u32; 3]> {
    let mut fields = def.trim_start_matches('#').split(';');
    fields.next()?;
    if fields.next()? != "2" {
        return None;
    }
    let mut rgb = [0u32; 3];
    for slot in &mut rgb {
        *slot = fields.next()?.parse().ok()?;
    }
    Some(rgb)
}

/// Blank out the pixels of one colour in a sixel and ask the terminal to
/// leave those positions alone (`P2 = 1`), so what is already on the
/// screen shows through them.
///
/// Sixel has no alpha channel, which is why transparency is flattened onto
/// a background colour before encoding; but the encoder's idea of that
/// colour is only ever a multiple of eight per channel, so the flattened
/// area comes out close to the terminal's background rather than equal to
/// it. The cells under a picture are blanked before it is printed, so
/// leaving those pixels unset shows the terminal's real background instead,
/// with none of the encoder's colour loss.
///
/// `rgb` has to be a colour the encoder can hit exactly (`sixel_snap`).
/// None when the picture has no such colour, and so nothing to drop.
pub fn sixel_drop_colour(data: &str, rgb: [u8; 3]) -> Option<String> {
    if !data.starts_with("\x1bP") {
        return None;
    }
    let b = data.as_bytes();
    let targets = indices_of_colour(data, sixel_percent(rgb));
    if targets.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(data.len());
    // P2 = 1: a position this data never sets keeps what the screen had
    out.push_str("\x1bP0;1;0q");
    let mut i = data.find('q')? + 1;
    // The chosen colour carries on across "$" and "-", so whole bands are
    // written with no "#n" in front of them at all; what is being drawn
    // has to be tracked rather than read off the run itself.
    let mut chosen = None;
    while i < b.len() {
        match b[i] {
            b'#' => {
                let mut j = i + 1;
                let mut index = 0u32;
                while j < b.len() && b[j].is_ascii_digit() {
                    index = index * 10 + u32::from(b[j] - b'0');
                    j += 1;
                }
                if j == i + 1 {
                    out.push('#');
                    i += 1;
                    continue;
                }
                if b.get(j) == Some(&b';') {
                    // "#n;2;r;g;b" defines a colour rather than choosing
                    // it; every definition is kept, so the indices the
                    // rest of the data selects still mean the same
                    while j < b.len() && (b[j].is_ascii_digit() || b[j] == b';') {
                        j += 1;
                    }
                } else {
                    chosen = Some(index);
                }
                out.push_str(&data[i..j]);
                i = j;
            }
            c if is_sixel_data(c) || c == b'!' => {
                let start = i;
                while i < b.len() && (is_sixel_data(b[i]) || b[i] == b'!' || b[i].is_ascii_digit())
                {
                    i += 1;
                }
                if chosen.is_some_and(|n| targets.contains(&n)) {
                    // The run stays where it is: colours are written one
                    // after another across a band, each carrying on from
                    // where the last one stopped, so taking a run out
                    // would pull everything after it leftwards. Only its
                    // data is replaced, by the sixel that sets no pixel
                    // and advances exactly as far.
                    for &c in &b[start..i] {
                        out.push(if is_sixel_data(c) { '?' } else { char::from(c) });
                    }
                } else {
                    out.push_str(&data[start..i]);
                }
            }
            c => {
                out.push(char::from(c));
                i += 1;
            }
        }
    }
    Some(out)
}

/// A byte that draws: one sixel is six pixels of a band, `?` none of them.
fn is_sixel_data(c: u8) -> bool {
    (0x3f..=0x7e).contains(&c)
}

/// Scale a picture to exactly `box_px`, ignoring its shape. Pictures in
/// chat are laid out within half a cell of their shape, so the stretch is
/// invisible; a protocol picture that does not fill its cells exactly would
/// be padded with black instead.
pub fn stretch_to(img: DynamicImage, box_px: (u32, u32)) -> DynamicImage {
    let (w, h) = (box_px.0.max(1), box_px.1.max(1));
    if (w, h) == (img.width(), img.height()) {
        img
    } else {
        img.resize_exact(w, h, FilterType::Triangle)
    }
}

/// Scale a picture so it covers `box_px` and crop the middle: how avatars
/// fill their block without being stretched.
pub fn cover_into(img: DynamicImage, box_px: (u32, u32)) -> DynamicImage {
    let (bw, bh) = (box_px.0.max(1), box_px.1.max(1));
    let (w, h) = (img.width().max(1), img.height().max(1));
    let scale = (bw as f64 / w as f64).max(bh as f64 / h as f64);
    let nw = ((w as f64 * scale).ceil() as u32).max(bw);
    let nh = ((h as f64 * scale).ceil() as u32).max(bh);
    let scaled = if (nw, nh) == (w, h) {
        img
    } else {
        img.resize_exact(nw, nh, FilterType::Triangle)
    };
    scaled.crop_imm((nw - bw) / 2, (nh - bh) / 2, bw, bh)
}

/// Blend a picture over a solid colour: for sixel, which has no
/// transparency, a round avatar sits on the theme's background.
pub fn composite_over(img: &RgbaImage, bg: [u8; 3]) -> RgbaImage {
    let mut out = img.clone();
    for px in out.pixels_mut() {
        let a = px.0[3] as u32;
        if a == 255 {
            continue;
        }
        for (channel, b) in px.0.iter_mut().take(3).zip(bg) {
            *channel = ((*channel as u32 * a + b as u32 * (255 - a)) / 255) as u8;
        }
        px.0[3] = 255;
    }
    out
}

/// The web app's default avatar for a user without a picture: one of six
/// on the static CDN, picked by id.
pub fn default_avatar_url(static_cdn: &str, user_id: &str) -> String {
    let idx = user_id
        .parse::<u128>()
        .map(|id| (id % DEFAULT_AVATAR_COLORS.len() as u128) as usize)
        .unwrap_or(0);
    format!("{}/avatars/{idx}.png?v=1", static_cdn.trim_end_matches('/'))
}

/// Thin an animation to at most `max` frames, evenly, keeping its length:
/// a dropped frame's time goes to the kept frame before it.
pub fn subsample(
    frames: Vec<DynamicImage>,
    delays: Vec<Duration>,
    max: usize,
) -> (Vec<DynamicImage>, Vec<Duration>) {
    let n = frames.len();
    if n <= max || max == 0 || delays.len() != n {
        return (frames, delays);
    }
    let keep: Vec<usize> = (0..max).map(|k| k * n / max).collect();
    let mut frames: Vec<Option<DynamicImage>> = frames.into_iter().map(Some).collect();
    let mut out_frames = Vec::with_capacity(max);
    let mut out_delays = Vec::with_capacity(max);
    for (k, &i) in keep.iter().enumerate() {
        let end = keep.get(k + 1).copied().unwrap_or(n);
        out_delays.push(delays[i..end].iter().sum());
        out_frames.push(frames[i].take().expect("each index kept once"));
    }
    (out_frames, out_delays)
}

#[cfg(test)]
mod sixel_alpha_tests {
    use super::*;

    /// `#002b36` is not a colour the encoder can hold: five bits a channel
    /// leaves only multiples of eight.
    #[test]
    fn snapping_lands_on_what_the_encoder_can_reproduce() {
        assert_eq!(sixel_snap([0x00, 0x2b, 0x36]), [0x00, 0x28, 0x30]);
        assert_eq!(sixel_snap([0xff, 0xff, 0xff]), [0xf8, 0xf8, 0xf8]);
        assert_eq!(
            sixel_snap([0x28, 0x30, 0x00]),
            [0x28, 0x30, 0x00],
            "already"
        );
    }

    #[test]
    fn the_flattened_colour_stops_drawing_and_the_rest_is_kept() {
        // #0 is the flattened-on colour, #1 the picture's own
        let data = "\x1bPq\"1;1;12;6#0;2;0;16;19#1;2;80;20;20#0!6~$#1!6~-#0~~~$#1~~~\x1b\\";
        let out = sixel_drop_colour(data, [0x00, 0x28, 0x30]).expect("that colour is in it");
        assert!(
            out.starts_with("\x1bP0;1;0q"),
            "asks for transparency: {out:?}"
        );
        assert!(
            !out.contains("#0!6~"),
            "the flattened run draws nothing: {out:?}"
        );
        assert!(!out.contains("#0~~~"), "in every band: {out:?}");
        assert!(out.contains("#0!6?"), "but still advances as far: {out:?}");
        assert!(out.contains("#0???"), "in every band: {out:?}");
        assert!(out.contains("#1!6~"), "the picture is kept: {out:?}");
        assert!(out.contains("#1~~~"), "in every band: {out:?}");
        assert_eq!(
            out.len() - "\x1bP0;1;0q".len(),
            data.len() - "\x1bPq".len(),
            "nothing was taken out, so nothing after it shifted: {out:?}"
        );
        assert!(
            out.contains("#0;2;0;16;19"),
            "definitions stay, so the indices still mean the same: {out:?}"
        );
        assert!(out.ends_with("\x1b\\"), "still terminated: {out:?}");
    }

    /// The chosen colour carries on across "$" and "-", so a band can draw
    /// without naming a colour at all; those pixels are the flattened
    /// colour just the same and have to stop drawing too. Real stickers
    /// come out this way: a run of bands that is nothing but the flattened
    /// colour is written once and then simply carried on.
    #[test]
    fn a_run_that_inherits_the_colour_is_blanked_too() {
        let data = "\x1bPq\"1;1;6;24#0;2;0;16;19#1;2;80;20;20#0!6~-!6~-#1!6~-!6~\x1b\\";
        let out = sixel_drop_colour(data, [0x00, 0x28, 0x30]).expect("that colour is in it");
        assert!(out.contains("#0!6?"), "the run that names it: {out:?}");
        assert!(
            out.contains("-!6?-"),
            "and the band that inherits it: {out:?}"
        );
        assert!(
            out.contains("#1!6~"),
            "the picture's own colour stays: {out:?}"
        );
        assert!(
            out.ends_with("-!6~\x1b\\"),
            "as does the band inheriting that: {out:?}"
        );
    }

    /// A "$" returns to the left of the same band without choosing a
    /// colour again, so the second pass draws in the first one's colour.
    #[test]
    fn a_second_pass_over_a_band_inherits_it_as_well() {
        let data = "\x1bPq\"1;1;6;6#1;2;80;20;20#0;2;0;16;19#0!6~$!6~\x1b\\";
        let out = sixel_drop_colour(data, [0x00, 0x28, 0x30]).expect("that colour is in it");
        assert!(
            out.contains("#0!6?$!6?"),
            "both passes stop drawing: {out:?}"
        );
    }

    #[test]
    fn a_picture_without_that_colour_is_left_alone() {
        let data = "\x1bPq\"1;1;6;6#1;2;80;20;20#1!6~\x1b\\";
        assert_eq!(sixel_drop_colour(data, [0x00, 0x28, 0x30]), None);
    }

    /// `#1` must not match `#12`: the whole run of digits is the index.
    #[test]
    fn an_index_is_not_a_prefix_of_another() {
        // 0x28 is written as 16 hundredths, and index 1 is a different grey
        let data = "\x1bPq\"1;1;6;6#1;2;20;20;20#12;2;16;16;16#1!6~$#12!6~\x1b\\";
        let out = sixel_drop_colour(data, [0x28, 0x28, 0x28]).expect("index 12 is in it");
        assert!(out.contains("#1!6~"), "index 1 is kept: {out:?}");
        assert!(!out.contains("#12!6~"), "index 12 draws nothing: {out:?}");
        assert!(out.contains("#12!6?"), "but still advances: {out:?}");
    }

    #[test]
    fn anything_that_is_not_a_sixel_is_refused() {
        assert_eq!(sixel_drop_colour("", [0, 0x28, 0x30]), None);
        assert_eq!(sixel_drop_colour("\x1b_Ga=T\x1b\\", [0, 0x28, 0x30]), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CELL: (u32, u32) = (10, 20);

    #[test]
    fn previews_keep_their_shape_and_never_grow() {
        // landscape 400x200 into 40x10 cells of 10x20 px (400x200 px): fits exactly
        assert_eq!(picture_cells((400, 200), CELL, (40, 10)), (40, 10));
        // tall: height limits -> 300 px tall, 150 wide -> 15 cols, 10 rows... at 200 px: 10 rows, 100 px wide = 10 cols
        assert_eq!(picture_cells((300, 600), CELL, (40, 10)), (10, 10));
        // wide: width limits -> 400x100 px -> 40 cols, 5 rows
        assert_eq!(picture_cells((1600, 400), CELL, (40, 10)), (40, 5));
        // small stays small: 35x25 px -> 4 cols, 1 row (nearest)
        assert_eq!(picture_cells((35, 25), CELL, (40, 10)), (4, 1));
        assert_eq!(picture_cells((0, 0), CELL, (40, 10)), (1, 1));
    }

    /// A block taller than the marker's four bits could not be drawn at
    /// all, so no box, however tall, yields one.
    #[test]
    fn a_block_is_never_taller_than_the_marker_can_carry() {
        // a square in a tall box: the rows stop at the cap, the shape holds
        assert_eq!(
            picture_cells((512, 512), CELL, (40, 36)),
            (BLOCK_MAX_ROWS * 2, BLOCK_MAX_ROWS)
        );
        assert_eq!(
            picture_cells((512, 2048), CELL, (40, 200)).1,
            BLOCK_MAX_ROWS
        );
        // a box within the cap is untouched
        assert_eq!(picture_cells((400, 200), CELL, (40, 10)), (40, 10));
    }

    #[test]
    fn preview_limits_follow_the_pane_and_the_pixel_cap() {
        // 10x20 px cells: 420 px = 42 cols, 300 px = 15 rows; a 30-row pane allows 10 rows
        assert_eq!(preview_limits(100, 30, CELL), (42, 10));
        // narrow pane
        assert_eq!(preview_limits(20, 30, CELL), (20, 10));
        // short pane: never under 3 rows
        assert_eq!(preview_limits(100, 6, CELL), (42, 3));
        // big console cells 15x31: 28 cols, 9 rows (pixel cap) even in a tall pane
        assert_eq!(preview_limits(100, 60, (15, 31)), (28, 9));
        // unknown cell size falls back to 8x16
        assert_eq!(preview_limits(100, 30, (0, 0)), (52, 10));
    }

    fn pic(w: u32, h: u32, animated: bool) -> InlinePicture {
        InlinePicture {
            proxy_url: "https://cdn.example/attachments/1/a.png".to_string(),
            width: w,
            height: h,
            animated,
        }
    }

    #[test]
    fn proxied_url_asks_for_the_limiting_side_only_when_smaller() {
        assert_eq!(
            proxied_url(&pic(1600, 400, false), (400, 200)),
            "https://cdn.example/attachments/1/a.png?width=400&format=webp"
        );
        assert_eq!(
            proxied_url(&pic(300, 600, true), (400, 200)),
            "https://cdn.example/attachments/1/a.png?height=200&format=webp&animated=true"
        );
        // already small enough: no resize asked for
        assert_eq!(
            proxied_url(&pic(100, 50, false), (400, 200)),
            "https://cdn.example/attachments/1/a.png?format=webp"
        );
        let mut p = pic(1000, 1000, true);
        p.proxy_url = "https://cdn.example/external/x/https/y.webp?sig=1".to_string();
        assert_eq!(
            proxied_url(&p, (300, 300)),
            "https://cdn.example/external/x/https/y.webp?sig=1&width=300&format=webp&animated=true"
        );
    }

    #[test]
    fn avatar_urls_follow_the_web_app() {
        assert_eq!(
            avatar_url("https://media.example/", None, "42", "a_deadbeef"),
            "https://media.example/avatars/42/deadbeef.webp?size=160"
        );
        assert_eq!(
            avatar_url("https://media.example", Some("7"), "42", "cafe"),
            "https://media.example/guilds/7/users/42/avatars/cafe.webp?size=160"
        );
        assert_eq!(
            avatar_url_sized(
                "https://media.example",
                None,
                "42",
                "a_cafe",
                AVATAR_PREVIEW_PX
            ),
            "https://media.example/avatars/42/cafe.webp?size=1024"
        );
    }

    #[test]
    fn default_avatar_colour_prefers_the_api_then_the_id() {
        let mut user: UserPartialResponse =
            serde_json::from_value(json!({"id": "7", "avatar_color": 0x3956d6})).unwrap();
        assert_eq!(default_avatar_color(&user), [0x39, 0x56, 0xd6]);
        user.avatar_color = None;
        assert_eq!(default_avatar_color(&user), [0xf0, 0xb1, 0x00]); // 7 % 6 = 1
        let key = default_avatar_key(default_avatar_color(&user));
        assert_eq!(key, "avatar-color:f0b100");
        assert_eq!(parse_default_avatar_key(&key), Some([0xf0, 0xb1, 0x00]));
        assert_eq!(parse_default_avatar_key("https://x"), None);
    }

    #[test]
    fn discs_are_round() {
        let img = disc_image([1, 2, 3], 32);
        assert_eq!(img.get_pixel(16, 16).0, [1, 2, 3, 255]);
        assert_eq!(img.get_pixel(0, 0).0[3], 0);
        assert_eq!(img.get_pixel(31, 31).0[3], 0);
        assert_eq!(img.get_pixel(16, 1).0[3], 255);
    }

    #[test]
    fn sixel_heights_are_whole_bands() {
        assert_eq!(sixel_rows(36), 36);
        assert_eq!(sixel_rows(40), 36);
        assert_eq!(sixel_rows(200), 198);
        assert_eq!(sixel_rows(5), 6);
    }

    #[test]
    fn covers_stretches_and_composites() {
        let covered = cover_into(DynamicImage::new_rgba8(300, 100), (40, 40));
        assert_eq!((covered.width(), covered.height()), (40, 40));
        let covered = cover_into(DynamicImage::new_rgba8(10, 10), (40, 38));
        assert_eq!((covered.width(), covered.height()), (40, 38));
        let stretched = stretch_to(DynamicImage::new_rgba8(233, 132), (240, 140));
        assert_eq!((stretched.width(), stretched.height()), (240, 140));
        let mut img = RgbaImage::from_pixel(2, 1, Rgba([255, 255, 255, 255]));
        img.put_pixel(1, 0, Rgba([255, 255, 255, 0]));
        let flat = composite_over(&img, [10, 20, 30]);
        assert_eq!(flat.get_pixel(0, 0).0, [255, 255, 255, 255]);
        assert_eq!(flat.get_pixel(1, 0).0, [10, 20, 30, 255]);
        assert_eq!(
            default_avatar_url("https://fluxerstatic.com/", "7"),
            "https://fluxerstatic.com/avatars/1.png?v=1"
        );
        assert_eq!(
            default_avatar_url("https://s", "x"),
            "https://s/avatars/0.png?v=1"
        );
    }

    #[test]
    fn thins_long_animations_evenly() {
        let frames: Vec<DynamicImage> = (0..10).map(|_| DynamicImage::new_rgba8(2, 2)).collect();
        let delays = vec![Duration::from_millis(10); 10];
        let (f, d) = subsample(frames, delays, 4);
        assert_eq!(f.len(), 4);
        assert_eq!(d.iter().sum::<Duration>(), Duration::from_millis(100));
        assert_eq!(d[0], Duration::from_millis(20)); // frames 0,1 -> kept 0; 2..5 -> 2 ...
        let (f, d) = subsample(
            vec![DynamicImage::new_rgba8(1, 1); 3],
            vec![Duration::ZERO; 3],
            8,
        );
        assert_eq!((f.len(), d.len()), (3, 3));
    }

    #[test]
    fn message_media_become_previews_only_with_a_proxy_and_a_size() {
        let a: MessageAttachmentResponse = serde_json::from_value(json!({
            "id": "1", "filename": "clip.mp4", "content_type": "video/mp4",
            "proxy_url": "https://cdn.example/attachments/1/clip.mp4", "width": 460, "height": 356
        }))
        .unwrap();
        let p = attachment_picture(&a).expect("video poster");
        assert_eq!((p.width, p.height, p.animated), (460, 356, false));
        let a: MessageAttachmentResponse = serde_json::from_value(json!({
            "id": "2", "filename": "dance.gif", "content_type": "image/gif",
            "proxy_url": "https://cdn.example/attachments/2/dance.gif", "width": 200, "height": 200
        }))
        .unwrap();
        assert!(attachment_picture(&a).unwrap().animated);
        let a: MessageAttachmentResponse = serde_json::from_value(json!({
            "id": "3", "filename": "notes.txt", "content_type": "text/plain",
            "proxy_url": "https://cdn.example/attachments/3/notes.txt", "width": 1, "height": 1
        }))
        .unwrap();
        assert!(attachment_picture(&a).is_none());
        let a: MessageAttachmentResponse = serde_json::from_value(json!({
            "id": "4", "filename": "old.png", "content_type": "image/png",
            "proxy_url": "https://cdn.example/attachments/4/old.png"
        }))
        .unwrap();
        assert!(attachment_picture(&a).is_none(), "no size known");

        let e: MessageEmbedResponse = serde_json::from_value(json!({
            "type": "gifv", "url": "https://klipy.com/gifs/x",
            "thumbnail": {"url": "https://static.klipy.com/a.webp", "proxy_url": "https://cdn.example/external/s/https/static.klipy.com/a.webp", "width": 312, "height": 312, "flags": 32},
            "video": {"url": "https://static.klipy.com/a.webm", "proxy_url": "https://cdn.example/external/t/https/static.klipy.com/a.webm", "width": 312, "height": 312}
        }))
        .unwrap();
        let p = embed_picture(&e).expect("gif thumbnail");
        assert!(p.animated);
        assert!(p.proxy_url.ends_with("a.webp"));
        let e: MessageEmbedResponse = serde_json::from_value(json!({
            "type": "link", "url": "https://example.com", "title": "Example",
            "thumbnail": {"url": "https://example.com/icon.png", "proxy_url": "https://cdn.example/external/u/https/example.com/icon.png", "width": 64, "height": 64}
        }))
        .unwrap();
        assert!(
            embed_picture(&e).is_none(),
            "a link's thumbnail is an icon, not a picture"
        );
    }
}
