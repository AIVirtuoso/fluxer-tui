//! Turning downloaded bytes into the pictures of one block of cells:
//! decode, thin long animations, scale to the block, round avatars, then
//! encode for the terminal's protocol or keep pixels for the console.

use crate::app::{MediaKind, MediaSlot, Picture, PictureFrames, terminal_picture};
use crate::media::gif_anim::decode_preview_animation;
use crate::media::inline::{
    Flatten, INLINE_MAX_FRAMES, block_px, circle_mask, composite_over, cover_into, disc_image,
    parse_default_avatar_key, sixel_rows, stretch_to, subsample,
};
use image::DynamicImage;
use ratatui::layout::Rect;
use ratatui_image::Resize;
use ratatui_image::picker::{Picker, ProtocolType};
use std::sync::Arc;
use std::time::Duration;

/// The pictures of a block, and the bytes of pixels they hold (what the
/// memory cache counts). None when the bytes are not a picture. `bytes` is
/// None for an avatar drawn locally.
pub fn prepare_pictures(
    bytes: Option<&[u8]>,
    slot: &MediaSlot,
    picker: Option<&Picker>,
    pixel_mode: bool,
    cell_px: (u32, u32),
    opaque_bg: Option<Flatten>,
) -> Option<(PictureFrames, usize)> {
    let (mut frames, mut delays) = match bytes {
        None => {
            let color = parse_default_avatar_key(&slot.url)?;
            (
                vec![DynamicImage::ImageRgba8(disc_image(color, 64))],
                vec![Duration::ZERO],
            )
        }
        Some(bytes) => match decode_preview_animation(bytes) {
            Some(animation) => animation,
            None => (
                vec![image::load_from_memory(bytes).ok()?],
                vec![Duration::ZERO],
            ),
        },
    };
    match slot.kind {
        MediaKind::Avatar => {
            frames.truncate(1);
            delays.truncate(1);
        }
        MediaKind::Picture => {
            if frames.len() > INLINE_MAX_FRAMES {
                (frames, delays) = subsample(frames, delays, INLINE_MAX_FRAMES);
            }
        }
    }
    // A round avatar needs transparency around it (pixels on the console,
    // kitty and iTerm2 have it) or a known colour to sit on: sixel has no
    // transparency, so there it is round only when the theme's background
    // is known, and square otherwise.
    let alpha_ok = pixel_mode
        || picker.is_some_and(|p| {
            matches!(
                p.protocol_type(),
                ProtocolType::Kitty | ProtocolType::Iterm2
            )
        });
    let round = slot.kind == MediaKind::Avatar && (alpha_ok || opaque_bg.is_some());
    // Exactly the block's pixel size: a protocol picture that does not fill
    // its cells would be padded with black by the encoder.
    let sixel =
        !pixel_mode && picker.is_some_and(|p| matches!(p.protocol_type(), ProtocolType::Sixel));
    let box_px = block_px(slot.cols, slot.rows, cell_px);
    let box_px = if sixel {
        (box_px.0, sixel_rows(box_px.1))
    } else {
        box_px
    };
    // Where the protocol has no alpha, transparency is flattened onto a
    // colour. On sixel the picture is then stopped from drawing wherever it
    // was see-through, so the terminal's own background shows there; the
    // colour is what is left under the parts that are only half see-through.
    let flat = opaque_bg.map(|f| f.colour);
    // What a run cut out of this picture has to repeat: flatten onto the
    // same colour, and stop drawing where the picture was see-through.
    let transparent = if sixel && !alpha_ok { opaque_bg } else { None };
    let area = Rect::new(0, 0, slot.cols, slot.rows);
    // Every frame at the size it will be drawn, before flattening writes
    // the alpha away.
    let shaped: Vec<image::RgbaImage> = frames
        .into_iter()
        .map(|img| {
            let img = match slot.kind {
                MediaKind::Avatar => cover_into(img, box_px),
                MediaKind::Picture => stretch_to(img, box_px),
            };
            let mut rgba = img.into_rgba8();
            if round {
                circle_mask(&mut rgba);
            }
            rgba
        })
        .collect();
    // A position is left undrawn only where every frame is see-through.
    // One that another frame paints has to be painted here as well, or
    // that frame's pixels stay on the screen underneath this one; painting
    // it is also what lets a frame replace the one before without the
    // cells being blanked first, which is what an animation blinks with.
    let undrawn: Arc<image::RgbaImage> = Arc::new(if shaped.len() > 1 {
        let mut all = shaped[0].clone();
        for frame in &shaped[1..] {
            for (seen, px) in all.pixels_mut().zip(frame.pixels()) {
                if px.0[3] != 0 {
                    seen.0[3] = 255;
                }
            }
        }
        all
    } else {
        shaped.first().cloned().unwrap_or_default()
    });
    let mut pictures = Vec::with_capacity(shaped.len());
    let mut total = 0usize;
    for rgba in shaped {
        let mut rgba = rgba;
        // a run of rows cut at the top is re-encoded from the picture as
        // it stands here
        let pixels = sixel.then(|| Arc::new(rgba.clone()));
        if !alpha_ok && let Some(bg) = flat {
            rgba = composite_over(&rgba, bg);
        }
        total += rgba.len();
        let picture = if pixel_mode {
            Picture::Pixels(Arc::new(rgba))
        } else {
            let protocol = picker?
                .new_protocol(DynamicImage::ImageRgba8(rgba), area, Resize::Fit(None))
                .ok()?;
            Picture::Terminal(Arc::new(terminal_picture(
                &protocol,
                pixels,
                sixel.then(|| undrawn.clone()),
                transparent,
            )?))
        };
        pictures.push(picture);
    }
    if pictures.is_empty() {
        return None;
    }
    let delays = delays
        .into_iter()
        .map(|d| d.max(Duration::from_millis(20)))
        .collect();
    Some((PictureFrames::new(pictures, delays), total))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::inline::default_avatar_key;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        image::RgbaImage::from_pixel(w, h, image::Rgba([9, 8, 7, 255]))
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn pictures_fill_their_block_exactly_on_the_console() {
        let slot = MediaSlot::new("https://x/a.png".to_string(), 10, 3, MediaKind::Picture);
        let (frames, bytes) =
            prepare_pictures(Some(&png(1000, 500)), &slot, None, true, (10, 20), None).unwrap();
        assert_eq!(frames.frames.len(), 1);
        match &frames.frames[0] {
            Picture::Pixels(img) => assert_eq!((img.width(), img.height()), (100, 60)),
            _ => panic!("console mode keeps pixels"),
        }
        assert_eq!(bytes, 100 * 60 * 4);
        assert!(!frames.is_animated());
    }

    #[test]
    fn avatars_are_square_cropped_round_discs_on_the_console() {
        let slot = MediaSlot::new("https://x/me.webp".to_string(), 4, 2, MediaKind::Avatar);
        let (frames, _) =
            prepare_pictures(Some(&png(300, 100)), &slot, None, true, (10, 20), None).unwrap();
        let Picture::Pixels(img) = &frames.frames[0] else {
            panic!()
        };
        assert_eq!((img.width(), img.height()), (40, 40));
        assert_eq!(img.get_pixel(0, 0).0[3], 0, "corner is transparent");
        assert_eq!(img.get_pixel(20, 20).0[3], 255);
        let local = MediaSlot::new(default_avatar_key([1, 2, 3]), 4, 2, MediaKind::Avatar);
        let (frames, _) = prepare_pictures(None, &local, None, true, (10, 20), None).unwrap();
        let Picture::Pixels(img) = &frames.frames[0] else {
            panic!()
        };
        assert_eq!(img.get_pixel(20, 20).0, [1, 2, 3, 255]);
    }

    /// One palette index per pixel, None where the data never set it.
    /// Enough of the format for what the encoder writes.
    fn decode_sixel(data: &str) -> (usize, usize, Vec<Option<u32>>) {
        let b = data.as_bytes();
        let mut i = data.find('q').expect("a sixel") + 1;
        let (mut w, mut h) = (0usize, 0usize);
        if b.get(i) == Some(&b'"') {
            i += 1;
            let start = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b';') {
                i += 1;
            }
            let raster: Vec<usize> = data[start..i]
                .split(';')
                .map(|n| n.parse().unwrap_or(0))
                .collect();
            if raster.len() >= 4 {
                (w, h) = (raster[2], raster[3]);
            }
        }
        let mut px = vec![None; w * h];
        let (mut x, mut band, mut colour) = (0usize, 0usize, 0u32);
        let put = |px: &mut Vec<Option<u32>>, x: usize, band: usize, c: u8, colour| {
            for k in 0..6 {
                if (c - 0x3f) & (1 << k) != 0 {
                    let y = band * 6 + k;
                    if x < w && y < h {
                        px[y * w + x] = Some(colour);
                    }
                }
            }
        };
        while i < b.len() {
            match b[i] {
                b'#' => {
                    let mut j = i + 1;
                    let mut n = 0u32;
                    while j < b.len() && b[j].is_ascii_digit() {
                        n = n * 10 + u32::from(b[j] - b'0');
                        j += 1;
                    }
                    if b.get(j) == Some(&b';') {
                        while j < b.len() && (b[j].is_ascii_digit() || b[j] == b';') {
                            j += 1;
                        }
                    } else {
                        colour = n;
                    }
                    i = j;
                }
                b'$' => {
                    x = 0;
                    i += 1;
                }
                b'-' => {
                    x = 0;
                    band += 1;
                    i += 1;
                }
                b'!' => {
                    let mut j = i + 1;
                    let mut n = 0usize;
                    while j < b.len() && b[j].is_ascii_digit() {
                        n = n * 10 + usize::from(b[j] - b'0');
                        j += 1;
                    }
                    if let Some(&c) = b.get(j) {
                        for _ in 0..n {
                            put(&mut px, x, band, c, colour);
                            x += 1;
                        }
                    }
                    i = j + 1;
                }
                c if (0x3f..=0x7e).contains(&c) => {
                    put(&mut px, x, band, c, colour);
                    x += 1;
                    i += 1;
                }
                _ => i += 1,
            }
        }
        (w, h, px)
    }

    /// Colours are written one after another across a band, each carrying
    /// on from where the last stopped, so a run that is taken out rather
    /// than blanked pulls every later colour of that band leftwards. This
    /// is the test that tells the two apart: every pixel of the flattened
    /// colour is unset afterwards, and no other pixel moves or changes.
    #[test]
    fn blanking_a_colour_leaves_every_other_pixel_where_it_was() {
        // stripes, so any shift shows up as a changed pixel
        // A sticker's shape: nothing at all in the bands top and bottom,
        // which the encoder writes once and then carries on, and stripes
        // in between so that any shift shows up as a changed pixel.
        let mut img = image::RgbaImage::from_pixel(80, 60, image::Rgba([0, 0, 0, 0]));
        for (x, y, px) in img.enumerate_pixels_mut() {
            if !(18..42).contains(&y) {
                continue;
            }
            *px = match x / 10 {
                0 | 4 => image::Rgba([200, 0, 0, 255]),
                1 | 5 => image::Rgba([0, 200, 0, 255]),
                2 | 6 => image::Rgba([0, 0, 0, 0]), // flattened onto the colour
                _ => image::Rgba([0, 0, 200, 255]),
            };
        }
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Sixel);
        // already snapped, so both runs hand the encoder the same picture
        let colour = [0, 0x2b, 0x36];
        let encode = |drop| {
            let slot = MediaSlot::new("https://x/s.png".to_string(), 8, 3, MediaKind::Picture);
            let (frames, _) = prepare_pictures(
                Some(&bytes),
                &slot,
                Some(&picker),
                false,
                (10, 20),
                Some(Flatten { colour, drop }),
            )
            .unwrap();
            let Picture::Terminal(tp) = &frames.frames[0] else {
                panic!()
            };
            tp.printout(0, 3, 20, None).unwrap().rows.remove(0).1
        };
        let plain = encode(false);
        let blanked = encode(true);
        assert!(
            blanked.starts_with("\x1bP0;1;0q"),
            "asks the terminal to leave unset positions alone: {:?}",
            &blanked[..12.min(blanked.len())]
        );
        let (w, h, before) = decode_sixel(&plain);
        let (w2, h2, after) = decode_sixel(&blanked);
        assert_eq!((w, h), (w2, h2), "same raster");
        let dropped = (0..before.len())
            .find(|&i| after[i].is_none() && before[i].is_some())
            .expect("something was blanked");
        let index = before[dropped];
        let mut unset = 0;
        for i in 0..before.len() {
            if before[i] == index {
                assert_eq!(after[i], None, "pixel {i} kept the flattened colour");
                unset += 1;
            } else {
                assert_eq!(after[i], before[i], "pixel {i} moved or changed colour");
            }
        }
        assert!(
            unset > 1000,
            "the whole flattened area went, not a run: {unset}"
        );
    }

    /// The backend skips blanking a cell the picture paints in full, so
    /// the map had better be right: a cell that is only partly painted,
    /// or that the picture never reaches because it was cut to whole
    #[test]
    fn the_picture_keeps_its_own_pixels_of_the_flattened_colour() {
        let flat = [0u8, 0x28, 0x30];
        // opaque throughout, and the left half is exactly the flattened
        // colour: nothing here is transparent, so nothing may be blanked
        let mut img = image::RgbaImage::from_pixel(80, 60, image::Rgba([250, 250, 250, 255]));
        for (x, _, px) in img.enumerate_pixels_mut() {
            if x < 40 {
                *px = image::Rgba([flat[0], flat[1], flat[2], 255]);
            }
        }
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Sixel);
        let slot = MediaSlot::new("https://x/f.png".to_string(), 8, 3, MediaKind::Picture);
        let (frames, _) = prepare_pictures(
            Some(&bytes),
            &slot,
            Some(&picker),
            false,
            (10, 20),
            Some(Flatten {
                colour: flat,
                drop: true,
            }),
        )
        .unwrap();
        let Picture::Terminal(tp) = &frames.frames[0] else {
            panic!()
        };
        let data = tp.printout(0, 3, 20, None).unwrap().rows.remove(0).1;
        let (w, h, px) = decode_sixel(&data);
        let unset = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .filter(|(x, y)| px[y * w + x].is_none())
            .count();
        assert_eq!(unset, 0, "an opaque picture leaves no pixel undrawn");
    }

    /// The whole of it, on a picture shaped like a real sticker: a
    /// see-through border, white in the middle, and a stripe of exactly the
    /// colour the transparency is flattened onto. Every see-through pixel
    /// must stop drawing so the terminal's background shows there, and
    /// every other pixel must draw, or what was on the screen before shows
    /// through the middle of the picture.
    #[test]
    fn every_see_through_pixel_stops_and_every_other_one_draws() {
        let flat = [0u8, 0x2b, 0x36];
        let mut img = image::RgbaImage::from_pixel(80, 60, image::Rgba([0, 0, 0, 0]));
        for (x, y, px) in img.enumerate_pixels_mut() {
            *px = match (x, y) {
                // a see-through border all the way round
                (x, y) if !(10..70).contains(&x) || !(12..48).contains(&y) => {
                    image::Rgba([0, 0, 0, 0])
                }
                // a stripe of the background's own colour, opaque
                (x, _) if x < 30 => image::Rgba([flat[0], flat[1], flat[2], 255]),
                // and white
                _ => image::Rgba([255, 255, 255, 255]),
            };
        }
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Sixel);
        let slot = MediaSlot::new("https://x/s.png".to_string(), 8, 3, MediaKind::Picture);
        let (frames, _) = prepare_pictures(
            Some(&bytes),
            &slot,
            Some(&picker),
            false,
            (10, 20),
            Some(Flatten {
                colour: flat,
                drop: true,
            }),
        )
        .unwrap();
        let Picture::Terminal(tp) = &frames.frames[0] else {
            panic!()
        };
        let data = tp.printout(0, 3, 20, None).unwrap().rows.remove(0).1;
        let (w, h, px) = decode_sixel(&data);
        assert_eq!((w, h), (80, 60), "the picture fills its block");
        let (mut wrongly_drawn, mut wrongly_blank) = (0, 0);
        for y in 0..h {
            for x in 0..w {
                let clear = img.get_pixel(x as u32, y as u32).0[3] == 0;
                match (clear, px[y * w + x].is_none()) {
                    (true, false) => wrongly_drawn += 1,
                    (false, true) => wrongly_blank += 1,
                    _ => {}
                }
            }
        }
        assert_eq!(wrongly_drawn, 0, "see-through pixels that still draw");
        assert_eq!(
            wrongly_blank, 0,
            "the picture's own pixels that stopped drawing"
        );
    }

    /// A run of rows cut at the top is re-encoded from the picture as it
    /// was before flattening, so it has to flatten again the same way. If
    /// it did not, the encoder would keep whatever sits under the alpha,
    /// which is black in most files.
    #[test]
    fn a_cut_run_flattens_and_blanks_the_same_way_as_the_whole() {
        let flat = [0u8, 0x2b, 0x36];
        let mut img = image::RgbaImage::from_pixel(80, 60, image::Rgba([200, 30, 30, 255]));
        for (x, y, px) in img.enumerate_pixels_mut() {
            if x < 20 || y < 10 {
                *px = image::Rgba([0, 0, 0, 0]);
            }
        }
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Sixel);
        let cut = |drop| {
            let slot = MediaSlot::new("https://x/r.png".to_string(), 8, 3, MediaKind::Picture);
            let (frames, _) = prepare_pictures(
                Some(&bytes),
                &slot,
                Some(&picker),
                false,
                (10, 20),
                Some(Flatten { colour: flat, drop }),
            )
            .unwrap();
            let Picture::Terminal(tp) = &frames.frames[0] else {
                panic!()
            };
            // rows 1..3, which starts inside a band and is re-encoded
            let data = tp
                .printout(1, 3, 20, Some(&picker))
                .unwrap()
                .rows
                .remove(0)
                .1;
            let (w, h, px) = decode_sixel(&data);
            (w, h, px.iter().filter(|p| p.is_none()).count())
        };

        // told to draw the flattened colour: the run covers every pixel
        let (w, h, blank) = cut(false);
        assert!(w > 0 && h > 0, "the run encoded");
        assert_eq!(blank, 0, "nothing is left undrawn when nothing is blanked");

        // told to blank it: only what was see-through stops drawing, and
        // rows 1..3 start at y = 20, past the see-through top strip
        let (_, _, blank) = cut(true);
        assert!(blank > 0, "the see-through left edge stops drawing");
        assert!(
            blank < w * h,
            "but the picture itself still draws: {blank} of {}",
            w * h
        );
    }

    /// What the terminal ends up with, worked through: the cells the
    /// picture does not cover are blanked, then the picture draws, and a
    /// position it never draws keeps what was there. Show one picture,
    /// then another over it, and nothing of the first may be left. This is
    /// the sticker picker scrolled from one sticker to the next.
    #[test]
    fn a_picture_shown_over_another_leaves_nothing_of_it() {
        const JUNK: u8 = 9;
        const BG: u8 = 0;
        let (cols, rows) = (8u16, 3u16);
        let cell = (10u32, 20u32);
        let mut picker = Picker::from_fontsize((cell.0 as u16, cell.1 as u16));
        picker.set_protocol_type(ProtocolType::Sixel);
        // two stickers of different shapes, each see-through around itself
        let sticker = |inset: u32| {
            let mut img = image::RgbaImage::from_pixel(80, 60, image::Rgba([0, 0, 0, 0]));
            for (x, y, px) in img.enumerate_pixels_mut() {
                if x >= inset && x < 80 - inset && y >= inset / 2 && y < 60 - inset / 2 {
                    *px = image::Rgba([200, 60, 60, 255]);
                }
            }
            let mut out = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
                .unwrap();
            out
        };
        let shown = |bytes: &[u8]| {
            let slot = MediaSlot::new(
                "https://x/p.png".to_string(),
                cols,
                rows,
                MediaKind::Picture,
            );
            let (frames, _) = prepare_pictures(
                Some(bytes),
                &slot,
                Some(&picker),
                false,
                cell,
                Some(Flatten {
                    colour: [0, 0x2b, 0x36],
                    drop: true,
                }),
            )
            .unwrap();
            let Picture::Terminal(tp) = &frames.frames[0] else {
                panic!()
            };
            let out = tp.printout(0, rows, cell.1, None).unwrap();
            (tp.area(), out.rows[0].1.clone())
        };

        let (w, h) = (80usize, 60usize);
        let mut screen = vec![JUNK; w * h];
        let mut drew_last = vec![false; w * h];
        for (mark, bytes) in [(1u8, sticker(4)), (2u8, sticker(16))] {
            let (area, data) = shown(&bytes);
            // the backend blanks every cell the picture is printed over
            for r in 0..area.height as usize {
                for c in 0..area.width as usize {
                    for y in r * cell.1 as usize..(r + 1) * cell.1 as usize {
                        for x in c * cell.0 as usize..(c + 1) * cell.0 as usize {
                            if x < w && y < h {
                                screen[y * w + x] = BG;
                            }
                        }
                    }
                }
            }
            // then the picture draws, and only where it draws
            let (dw, dh, px) = decode_sixel(&data);
            drew_last.fill(false);
            for y in 0..dh.min(h) {
                for x in 0..dw.min(w) {
                    if px[y * dw + x].is_some() {
                        screen[y * w + x] = mark;
                        drew_last[y * w + x] = true;
                    }
                }
            }
        }
        let stale = (0..w * h)
            .filter(|i| !drew_last[*i] && screen[*i] != BG)
            .count();
        assert_eq!(
            stale, 0,
            "pixels the second sticker does not draw still show something older"
        );
    }

    /// The contract the backend leans on: a cell said to be covered is not
    /// blanked before the picture is printed, so the picture had better
    /// paint every pixel of it. Where that is not true the cell keeps what
    /// The whole of the animation rule: a position no frame ever draws is
    /// left undrawn, so the terminal's own background shows through it
    /// exactly; one that another frame paints is painted here too, so that
    /// showing this frame replaces that one outright and the cells never
    /// have to be blanked between frames.
    #[test]
    fn a_frame_paints_whatever_any_other_frame_paints() {
        // two frames: a block on the left, then the same on the right
        let frame = |left: bool| {
            let mut img = image::RgbaImage::from_pixel(80, 60, image::Rgba([0, 0, 0, 0]));
            for (x, _, px) in img.enumerate_pixels_mut() {
                let inside = if left {
                    (8..36).contains(&x)
                } else {
                    (44..72).contains(&x)
                };
                if inside {
                    *px = image::Rgba([200, 60, 200, 255]);
                }
            }
            image::Frame::new(img)
        };
        let mut gif = Vec::new();
        {
            let mut enc = image::codecs::gif::GifEncoder::new(&mut gif);
            enc.encode_frames(vec![frame(true), frame(false)]).unwrap();
        }
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Sixel);
        let slot = MediaSlot::new("https://x/a.gif".to_string(), 8, 3, MediaKind::Picture);
        let (frames, _) = prepare_pictures(
            Some(&gif),
            &slot,
            Some(&picker),
            false,
            (10, 20),
            Some(Flatten {
                colour: [0, 0x2b, 0x36],
                drop: true,
            }),
        )
        .unwrap();
        assert_eq!(frames.frames.len(), 2, "both frames decoded");
        let drawn = |i: usize| {
            let Picture::Terminal(tp) = &frames.frames[i] else {
                panic!()
            };
            let g = tp.area();
            let data = tp.printout(0, g.height, 20, None).unwrap().rows[0]
                .1
                .clone();
            let (w, h, px) = decode_sixel(&data);
            (w, h, px)
        };
        let (w, h, a) = drawn(0);
        let (w2, h2, b) = drawn(1);
        assert_eq!((w, h), (w2, h2));
        let (mut differ, mut both_drawn, mut both_undrawn) = (0, 0, 0);
        for i in 0..w * h {
            match (a[i].is_some(), b[i].is_some()) {
                (true, true) => both_drawn += 1,
                (false, false) => both_undrawn += 1,
                _ => differ += 1,
            }
        }
        // GIF carries transparency as one palette entry, and quantising a
        // hard edge can move a pixel of it from one frame to the next, so a
        // hairline along the blocks' edges is the encoder's and not the
        // rule's: eight positions of 4800 here.
        assert!(
            differ * 200 <= w * h,
            "{differ} positions are drawn by one frame and not the other, so that \
             frame's pixels stay on the screen under this one"
        );
        assert!(
            both_drawn > 1000,
            "the blocks are painted in both: {both_drawn}"
        );
        assert!(
            both_undrawn > 500,
            "and what neither ever draws is left undrawn: {both_undrawn}"
        );
    }

    #[test]
    fn terminal_mode_encodes_for_the_protocol_at_the_block_size() {
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Sixel);
        let slot = MediaSlot::new("https://x/a.png".to_string(), 8, 3, MediaKind::Picture);
        let (frames, _) = prepare_pictures(
            Some(&png(800, 600)),
            &slot,
            Some(&picker),
            false,
            (10, 20),
            None,
        )
        .unwrap();
        match &frames.frames[0] {
            Picture::Terminal(tp) => {
                assert_eq!(tp.area(), Rect::new(0, 0, 8, 3), "fills its cells exactly");
                let out = tp.printout(0, 3, 20, None).expect("the whole block");
                assert_eq!(
                    out.rows.len(),
                    1,
                    "sixel draws everything from the first row"
                );
                assert_eq!(out.rows[0].0, 0);
                let data = &*out.rows[0].1;
                assert!(data.starts_with("\x1bP"), "a sixel sequence");
                // 3 rows of 20 px = 60 px = 10 whole bands: nothing spills below
                assert!(data.contains("\"1;1;80;60"), "{data:?}");
                assert_eq!(data.matches('-').count(), 9, "ten bands: {data:?}");
            }
            _ => panic!("terminal mode encodes a protocol"),
        }
        // 2 rows of 20 px = 40 px would end in a partial band: cut to 36
        let avatar = MediaSlot::new("https://x/me.png".to_string(), 4, 2, MediaKind::Avatar);
        let (frames, _) = prepare_pictures(
            Some(&png(100, 100)),
            &avatar,
            Some(&picker),
            false,
            (10, 20),
            None,
        )
        .unwrap();
        let Picture::Terminal(tp) = &frames.frames[0] else {
            panic!()
        };
        let data = tp.printout(0, 2, 20, None).unwrap().rows.remove(0).1;
        assert!(data.contains("\"1;1;40;36"), "{data:?}");
        assert_eq!(data.matches('-').count(), 5);
        // sixel carries no alpha of its own: an avatar is round there only
        // when there is a colour to flatten its corners onto
        let flat = |drop| {
            Some(Flatten {
                colour: [1, 2, 3],
                drop,
            })
        };
        for bg in [None, flat(false), flat(true)] {
            let (frames, _) = prepare_pictures(
                Some(&png(100, 100)),
                &avatar,
                Some(&picker),
                false,
                (10, 20),
                bg,
            )
            .unwrap();
            let Picture::Terminal(tp) = &frames.frames[0] else {
                panic!()
            };
            assert_eq!(tp.area(), Rect::new(0, 0, 4, 2));
        }
    }

    #[test]
    fn junk_and_missing_pickers_yield_nothing() {
        let slot = MediaSlot::new("https://x/a.png".to_string(), 10, 5, MediaKind::Picture);
        assert!(
            prepare_pictures(Some(b"not a picture"), &slot, None, true, (10, 20), None).is_none()
        );
        // terminal mode without a picker cannot encode
        assert!(prepare_pictures(Some(&png(4, 4)), &slot, None, false, (10, 20), None).is_none());
        assert!(prepare_pictures(None, &slot, None, true, (10, 20), None).is_none());
    }
}
