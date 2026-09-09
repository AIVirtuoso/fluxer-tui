//! Turning downloaded bytes into the pictures of one block of cells:
//! decode, thin long animations, scale to the block, round avatars, then
//! encode for the terminal's protocol or keep pixels for the console.

use crate::app::{MediaKind, MediaSlot, Picture, PictureFrames, terminal_picture};
use crate::media::gif_anim::decode_preview_animation;
use crate::media::inline::{
    Flatten, INLINE_MAX_FRAMES, block_px, circle_mask, composite_over, cover_into, disc_image,
    parse_default_avatar_key, sixel_rows, sixel_snap, stretch_to, subsample,
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
    // colour. On sixel those pixels are then dropped from the data, and
    // the colour is snapped to one the encoder can hit exactly so that
    // every one of them is recognisable as the same palette entry.
    let drop_flat = sixel && opaque_bg.is_some_and(|f| f.drop);
    let flat = opaque_bg.map(|f| {
        if drop_flat {
            sixel_snap(f.colour)
        } else {
            f.colour
        }
    });
    let transparent = if drop_flat { flat } else { None };
    let area = Rect::new(0, 0, slot.cols, slot.rows);
    let mut pictures = Vec::with_capacity(frames.len());
    let mut total = 0usize;
    for img in frames {
        let img = match slot.kind {
            MediaKind::Avatar => cover_into(img, box_px),
            MediaKind::Picture => stretch_to(img, box_px),
        };
        let mut rgba = img.into_rgba8();
        if round {
            circle_mask(&mut rgba);
        }
        if !alpha_ok && let Some(bg) = flat {
            rgba = composite_over(&rgba, bg);
        }
        total += rgba.len();
        let picture = if pixel_mode {
            Picture::Pixels(Arc::new(rgba))
        } else {
            // sixel keeps the pixels: a run of rows cut at the top is
            // encoded from them when it is first shown
            let pixels = sixel.then(|| Arc::new(rgba.clone()));
            let protocol = picker?
                .new_protocol(DynamicImage::ImageRgba8(rgba), area, Resize::Fit(None))
                .ok()?;
            Picture::Terminal(Arc::new(terminal_picture(&protocol, pixels, transparent)?))
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
        let colour = sixel_snap([0, 0x2b, 0x36]);
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
