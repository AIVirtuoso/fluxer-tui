//! Turning downloaded bytes into the pictures of one block of cells:
//! decode, thin long animations, scale to the block, round avatars, then
//! encode for the terminal's protocol or keep pixels for the console.

use crate::app::{MediaKind, MediaSlot, Picture, PictureFrames};
use crate::media::gif_anim::decode_preview_animation;
use crate::media::inline::{
    INLINE_MAX_FRAMES, block_px, circle_mask, disc_image, fit_into, parse_default_avatar_key,
    square_crop, subsample,
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
    // A round avatar needs transparency around it: pixels on the console,
    // kitty and iTerm2 have it; sixel has not, so avatars stay square there.
    let round = slot.kind == MediaKind::Avatar
        && (pixel_mode
            || picker.is_some_and(|p| {
                matches!(
                    p.protocol_type(),
                    ProtocolType::Kitty | ProtocolType::Iterm2
                )
            }));
    let box_px = block_px(slot.cols, slot.rows, cell_px);
    let area = Rect::new(0, 0, slot.cols, slot.rows);
    let mut pictures = Vec::with_capacity(frames.len());
    let mut total = 0usize;
    for img in frames {
        let img = if slot.kind == MediaKind::Avatar {
            square_crop(img)
        } else {
            img
        };
        let mut rgba = fit_into(img, box_px).into_rgba8();
        if round {
            circle_mask(&mut rgba);
        }
        total += rgba.len();
        let picture = if pixel_mode {
            Picture::Pixels(Arc::new(rgba))
        } else {
            let protocol = picker?
                .new_protocol(DynamicImage::ImageRgba8(rgba), area, Resize::Fit(None))
                .ok()?;
            Picture::Protocol(protocol)
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
    fn pictures_are_scaled_to_the_block_on_the_console() {
        let slot = MediaSlot::new("https://x/a.png".to_string(), 10, 5, MediaKind::Picture);
        let (frames, bytes) =
            prepare_pictures(Some(&png(1000, 500)), &slot, None, true, (10, 20)).unwrap();
        assert_eq!(frames.frames.len(), 1);
        match &frames.frames[0] {
            Picture::Pixels(img) => assert_eq!((img.width(), img.height()), (100, 50)),
            _ => panic!("console mode keeps pixels"),
        }
        assert_eq!(bytes, 100 * 50 * 4);
        assert!(!frames.is_animated());
    }

    #[test]
    fn avatars_are_square_cropped_round_discs_on_the_console() {
        let slot = MediaSlot::new("https://x/me.webp".to_string(), 4, 2, MediaKind::Avatar);
        let (frames, _) =
            prepare_pictures(Some(&png(300, 100)), &slot, None, true, (10, 20)).unwrap();
        let Picture::Pixels(img) = &frames.frames[0] else {
            panic!()
        };
        assert_eq!((img.width(), img.height()), (40, 40));
        assert_eq!(img.get_pixel(0, 0).0[3], 0, "corner is transparent");
        assert_eq!(img.get_pixel(20, 20).0[3], 255);
        let local = MediaSlot::new(default_avatar_key([1, 2, 3]), 4, 2, MediaKind::Avatar);
        let (frames, _) = prepare_pictures(None, &local, None, true, (10, 20)).unwrap();
        let Picture::Pixels(img) = &frames.frames[0] else {
            panic!()
        };
        assert_eq!(img.get_pixel(20, 20).0, [1, 2, 3, 255]);
    }

    #[test]
    fn terminal_mode_encodes_for_the_protocol_at_the_block_size() {
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Sixel);
        let slot = MediaSlot::new("https://x/a.png".to_string(), 8, 3, MediaKind::Picture);
        let (frames, _) =
            prepare_pictures(Some(&png(800, 600)), &slot, Some(&picker), false, (10, 20)).unwrap();
        match &frames.frames[0] {
            Picture::Protocol(p) => {
                let area = p.area();
                assert!(area.width <= 8 && area.height <= 3, "{area:?}");
                assert!(area.width >= 7 && area.height >= 2, "{area:?}");
            }
            _ => panic!("terminal mode encodes a protocol"),
        }
        // sixel has no transparency: avatars stay square there
        let avatar = MediaSlot::new("https://x/me.png".to_string(), 4, 2, MediaKind::Avatar);
        assert!(
            prepare_pictures(
                Some(&png(100, 100)),
                &avatar,
                Some(&picker),
                false,
                (10, 20)
            )
            .is_some()
        );
    }

    #[test]
    fn junk_and_missing_pickers_yield_nothing() {
        let slot = MediaSlot::new("https://x/a.png".to_string(), 10, 5, MediaKind::Picture);
        assert!(prepare_pictures(Some(b"not a picture"), &slot, None, true, (10, 20)).is_none());
        // terminal mode without a picker cannot encode
        assert!(prepare_pictures(Some(&png(4, 4)), &slot, None, false, (10, 20)).is_none());
        assert!(prepare_pictures(None, &slot, None, true, (10, 20)).is_none());
    }
}
