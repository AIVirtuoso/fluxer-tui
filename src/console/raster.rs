//! Cell buffer to pixels.

use anyhow::{Context, Result};
use image::RgbaImage;
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use std::collections::HashMap;
use std::sync::Arc;
use swash::scale::image::Content;
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::shape::ShapeContext;
use swash::text::Script;
use swash::{CacheKey, FontRef};
use unicode_width::UnicodeWidthStr;

/// A loaded font file; `FontRef`s are re-derived from it (cheap).
struct Font {
    data: Vec<u8>,
    offset: u32,
    key: CacheKey,
}

impl Font {
    fn load(path: &std::path::Path) -> Result<Self> {
        let data =
            std::fs::read(path).with_context(|| format!("reading font {}", path.display()))?;
        let font = FontRef::from_index(&data, 0)
            .with_context(|| format!("{} is not a usable font", path.display()))?;
        let (offset, key) = (font.offset, font.key);
        Ok(Self { data, offset, key })
    }

    fn as_ref(&self) -> FontRef<'_> {
        FontRef {
            data: &self.data,
            offset: self.offset,
            key: self.key,
        }
    }
}

/// One rendered glyph run for a cell symbol, in cell-relative pixels.
#[derive(Clone)]
struct Rendered {
    /// (x offset, y offset from the cell top, image)
    parts: Vec<(i32, i32, Arc<GlyphImage>)>,
}

struct GlyphImage {
    width: u32,
    height: u32,
    /// Alpha mask (len w*h) or RGBA (len w*h*4)
    color: bool,
    data: Vec<u8>,
}

/// A picture to blit over a cell rectangle after the text is drawn.
#[derive(Debug)]
pub struct Placement {
    pub area: Rect,
    pub image: Arc<RgbaImage>,
}

/// Default 16-colour palette for `Color::Indexed`/named colours on the
/// console (no terminal theme exists there): the Tango-like set most
/// terminals ship, which reads well on the dark default background.
pub const PALETTE16: [[u8; 3]; 16] = [
    [0x2e, 0x34, 0x36],
    [0xcc, 0x00, 0x00],
    [0x4e, 0x9a, 0x06],
    [0xc4, 0xa0, 0x00],
    [0x34, 0x65, 0xa4],
    [0x75, 0x50, 0x7b],
    [0x06, 0x98, 0x9a],
    [0xd3, 0xd7, 0xcf],
    [0x55, 0x57, 0x53],
    [0xef, 0x29, 0x29],
    [0x8a, 0xe2, 0x34],
    [0xfc, 0xe9, 0x4f],
    [0x72, 0x9f, 0xcf],
    [0xad, 0x7f, 0xa8],
    [0x34, 0xe2, 0xe2],
    [0xee, 0xee, 0xec],
];

pub const DEFAULT_FG: [u8; 3] = [0xd3, 0xd7, 0xcf];
pub const DEFAULT_BG: [u8; 3] = [0x00, 0x00, 0x00];

pub struct Rasterizer {
    text: Font,
    bold: Option<Font>,
    emoji: Option<Font>,
    scale: ScaleContext,
    shape: ShapeContext,
    px: f32,
    pub cell_w: u32,
    pub cell_h: u32,
    baseline: i32,
    cache: HashMap<(String, bool), Rendered>,
    pub fg: [u8; 3],
    pub bg: [u8; 3],
}

impl Rasterizer {
    pub fn new(fonts: &super::fonts::FontPaths, px: f32) -> Result<Self> {
        let text = Font::load(fonts.text.as_deref().context("no text font")?)?;
        let bold = fonts.bold.as_deref().map(Font::load).transpose()?;
        let emoji = fonts.emoji.as_deref().map(Font::load).transpose()?;
        let metrics = text.as_ref().metrics(&[]).scale(px);
        let cell_h = (metrics.ascent + metrics.descent + metrics.leading)
            .ceil()
            .max(1.0) as u32;
        let baseline = metrics.ascent.round() as i32;
        // cell width: the advance of a typical glyph
        let gm = text.as_ref().glyph_metrics(&[]).scale(px);
        let zero = text.as_ref().charmap().map('0');
        let mut cell_w = gm.advance_width(zero).round() as u32;
        if cell_w == 0 {
            cell_w = (metrics.average_width.round() as u32).max(1);
        }
        Ok(Self {
            text,
            bold,
            emoji,
            scale: ScaleContext::new(),
            shape: ShapeContext::new(),
            px,
            cell_w,
            cell_h,
            baseline,
            cache: HashMap::new(),
            fg: DEFAULT_FG,
            bg: DEFAULT_BG,
        })
    }

    /// Screen size in cells for a pixel size.
    pub fn grid(&self, width_px: u32, height_px: u32) -> (u16, u16) {
        (
            (width_px / self.cell_w).clamp(1, u16::MAX as u32) as u16,
            (height_px / self.cell_h).clamp(1, u16::MAX as u32) as u16,
        )
    }

    fn rgb(&self, c: Color, is_fg: bool) -> [u8; 3] {
        match c {
            Color::Reset => {
                if is_fg {
                    self.fg
                } else {
                    self.bg
                }
            }
            Color::Rgb(r, g, b) => [r, g, b],
            Color::Black => PALETTE16[0],
            Color::Red => PALETTE16[1],
            Color::Green => PALETTE16[2],
            Color::Yellow => PALETTE16[3],
            Color::Blue => PALETTE16[4],
            Color::Magenta => PALETTE16[5],
            Color::Cyan => PALETTE16[6],
            Color::Gray => PALETTE16[7],
            Color::DarkGray => PALETTE16[8],
            Color::LightRed => PALETTE16[9],
            Color::LightGreen => PALETTE16[10],
            Color::LightYellow => PALETTE16[11],
            Color::LightBlue => PALETTE16[12],
            Color::LightMagenta => PALETTE16[13],
            Color::LightCyan => PALETTE16[14],
            Color::White => PALETTE16[15],
            Color::Indexed(i) => indexed_rgb(i),
        }
    }

    fn wants_emoji_font(symbol: &str) -> bool {
        let mut chars = symbol.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        let cp = first as u32;
        let emoji_range = (0x1F000..=0x1FAFF).contains(&cp)
            || (0x2600..=0x27BF).contains(&cp)
            || (0x2300..=0x23FF).contains(&cp)
            || (0x2B00..=0x2BFF).contains(&cp)
            || (0x1F1E6..=0x1F1FF).contains(&cp);
        emoji_range || symbol.contains('\u{fe0f}') || symbol.contains('\u{200d}')
    }

    /// Render a cell symbol (one grapheme) with the right font, cached.
    fn rendered(&mut self, symbol: &str, bold: bool) -> Rendered {
        let key = (symbol.to_string(), bold);
        if let Some(r) = self.cache.get(&key) {
            return r.clone();
        }
        let width_cells = symbol.width().max(1) as u32;
        let box_w = self.cell_w * width_cells;
        let box_h = self.cell_h;

        let use_emoji = self.emoji.is_some() && Self::wants_emoji_font(symbol);
        let mut parts = Vec::new();
        let mut done = false;
        if use_emoji {
            parts = self.shape_and_render(symbol, FontChoice::Emoji, box_w, box_h);
            done = !parts.is_empty();
        }
        if !done {
            let choice = if bold && self.bold.is_some() {
                FontChoice::Bold
            } else {
                FontChoice::Text
            };
            parts = self.shape_and_render(symbol, choice, box_w, box_h);
            // fall back to the emoji font for anything the text font lacks
            if parts.is_empty() && self.emoji.is_some() && !use_emoji {
                parts = self.shape_and_render(symbol, FontChoice::Emoji, box_w, box_h);
            }
        }
        let r = Rendered { parts };
        self.cache.insert(key, r.clone());
        r
    }

    fn shape_and_render(
        &mut self,
        symbol: &str,
        choice: FontChoice,
        box_w: u32,
        box_h: u32,
    ) -> Vec<(i32, i32, Arc<GlyphImage>)> {
        let font = match choice {
            FontChoice::Text => &self.text,
            FontChoice::Bold => self.bold.as_ref().unwrap_or(&self.text),
            FontChoice::Emoji => match self.emoji.as_ref() {
                Some(f) => f,
                None => &self.text,
            },
        };
        let fref = font.as_ref();
        let charmap = fref.charmap();
        // anything the font cannot map at all: give up so the caller can
        // try another font
        let mut mapped_any = false;
        for ch in symbol.chars() {
            if ch == '\u{fe0f}' || ch == '\u{200d}' || ch == '\u{fe0e}' {
                continue;
            }
            if charmap.map(ch) != 0 {
                mapped_any = true;
            } else if !is_modifier_like(ch) {
                return Vec::new();
            }
        }
        if !mapped_any {
            return Vec::new();
        }

        // shape (handles ZWJ sequences, skin tones, variation selectors)
        let mut shaper = self
            .shape
            .builder(fref)
            .script(Script::Latin)
            .size(self.px)
            .build();
        shaper.add_str(symbol);
        let mut glyphs: Vec<(u16, f32, f32)> = Vec::new();
        let mut pen = 0f32;
        shaper.shape_with(|cluster| {
            for g in cluster.glyphs {
                glyphs.push((g.id, pen + g.x, g.y));
                pen += g.advance;
            }
        });
        if glyphs.is_empty() {
            return Vec::new();
        }

        let mut scaler = self.scale.builder(fref).size(self.px).hint(true).build();
        let render = Render::new(&[
            Source::ColorBitmap(StrikeWith::BestFit),
            Source::ColorOutline(0),
            Source::Outline,
        ]);
        let mut out = Vec::new();
        for (id, gx, gy) in glyphs {
            let Some(img) = render.render(&mut scaler, id) else {
                continue;
            };
            if img.placement.width == 0 || img.placement.height == 0 {
                continue;
            }
            let (w, h) = (img.placement.width, img.placement.height);
            match img.content {
                Content::Mask => {
                    let x = gx.round() as i32 + img.placement.left;
                    let y = self.baseline - img.placement.top - gy.round() as i32;
                    out.push((
                        x,
                        y,
                        Arc::new(GlyphImage {
                            width: w,
                            height: h,
                            color: false,
                            data: img.data,
                        }),
                    ));
                }
                Content::Color | Content::SubpixelMask => {
                    // bitmap strikes come at their own size: fit them into the
                    // cell box, keeping proportions, and centre
                    let mut rgba = RgbaImage::from_raw(w, h, img.data).unwrap_or_default();
                    let scale = (box_w as f32 / w as f32)
                        .min(box_h as f32 / h as f32)
                        .min(1.0e6);
                    let (nw, nh) = (
                        ((w as f32 * scale).round() as u32).max(1),
                        ((h as f32 * scale).round() as u32).max(1),
                    );
                    if (nw, nh) != (w, h) {
                        rgba = image::imageops::resize(
                            &rgba,
                            nw,
                            nh,
                            image::imageops::FilterType::Triangle,
                        );
                    }
                    let x = ((box_w - nw) / 2) as i32;
                    let y = ((box_h - nh) / 2) as i32;
                    out.push((
                        x,
                        y,
                        Arc::new(GlyphImage {
                            width: nw,
                            height: nh,
                            color: true,
                            data: rgba.into_raw(),
                        }),
                    ));
                }
            }
        }
        out
    }

    /// Paint `buf` (plus pictures and the cursor) into an XRGB8888 frame of
    /// `width x height` pixels with `stride` pixels per row.
    #[allow(clippy::too_many_arguments)] // a buffer and the frame it is painted into
    pub fn render(
        &mut self,
        buf: &Buffer,
        placements: &[Placement],
        cursor: Option<(u16, u16)>,
        frame: &mut [u32],
        width: u32,
        height: u32,
        stride: u32,
    ) {
        let bgpx = pack(self.bg);
        for row in 0..height {
            let start = (row * stride) as usize;
            frame[start..start + width as usize].fill(bgpx);
        }
        let area = buf.area;
        for y in area.top()..area.bottom() {
            let mut x = area.left();
            while x < area.right() {
                let w = self.paint_cell(&buf[(x, y)], x, y, frame, width, height, stride, false);
                x += w as u16;
            }
        }
        self.finish(placements, cursor, frame, width, height, stride);
    }

    /// Paint only the cells marked in `dirty` (one flag per cell of `buf`),
    /// with their neighbours, since a glyph may reach into the next cell;
    /// then the pictures and the cursor, over whatever they cover.
    #[allow(clippy::too_many_arguments)]
    pub fn render_cells(
        &mut self,
        buf: &Buffer,
        dirty: &[bool],
        placements: &[Placement],
        cursor: Option<(u16, u16)>,
        frame: &mut [u32],
        width: u32,
        height: u32,
        stride: u32,
    ) {
        let area = buf.area;
        let cols = area.width as usize;
        for y in area.top()..area.bottom() {
            let Some(row) = dirty.get(y as usize * cols..(y as usize + 1) * cols) else {
                break;
            };
            if !row.iter().any(|&d| d) {
                continue;
            }
            let mut x = area.left();
            while x < area.right() {
                let cell = &buf[(x, y)];
                let w = cell.symbol().width().max(1);
                let lo = (x as usize).saturating_sub(1);
                let hi = (x as usize + w + 1).min(cols);
                if row[lo..hi].iter().any(|&d| d) {
                    self.paint_cell(cell, x, y, frame, width, height, stride, true);
                }
                x += w as u16;
            }
        }
        self.finish(placements, cursor, frame, width, height, stride);
    }

    fn finish(
        &self,
        placements: &[Placement],
        cursor: Option<(u16, u16)>,
        frame: &mut [u32],
        width: u32,
        height: u32,
        stride: u32,
    ) {
        for p in placements {
            self.blit_placement(frame, width, height, stride, p);
        }
        if let Some((cx, cy)) = cursor {
            let (cw, ch) = (self.cell_w, self.cell_h);
            let px0 = cx as u32 * cw;
            let py0 = cy as u32 * ch;
            // underline-style cursor, two pixels tall, in the default fg
            fill_rect(
                frame,
                stride,
                width,
                height,
                px0,
                py0 + ch.saturating_sub(2),
                cw,
                2,
                self.fg,
            );
        }
    }

    /// Paint one cell: its background (always when `erase`, else only when
    /// it differs from the frame's), the glyph, underline and strike-through.
    /// Returns how many cells the symbol spans.
    #[allow(clippy::too_many_arguments)]
    fn paint_cell(
        &mut self,
        cell: &Cell,
        x: u16,
        y: u16,
        frame: &mut [u32],
        width: u32,
        height: u32,
        stride: u32,
        erase: bool,
    ) -> u32 {
        let bg = self.bg;
        let (cw, ch) = (self.cell_w, self.cell_h);
        let symbol = cell.symbol();
        let wcells = symbol.width().max(1) as u32;
        let px0 = x as u32 * cw;
        let py0 = y as u32 * ch;
        if px0 >= width || py0 >= height {
            return wcells;
        }
        let (mut fg, mut cbg) = self.cell_colors(cell);
        if cell.modifier.contains(Modifier::REVERSED) {
            std::mem::swap(&mut fg, &mut cbg);
        }
        if cell.modifier.contains(Modifier::DIM) {
            fg = mix(fg, cbg, 0.55);
        }
        // background
        if erase || cbg != bg {
            fill_rect(frame, stride, width, height, px0, py0, cw * wcells, ch, cbg);
        }
        // glyph
        if !symbol.trim().is_empty() && !cell.modifier.contains(Modifier::HIDDEN) {
            let r = self.rendered(symbol, cell.modifier.contains(Modifier::BOLD));
            let synth_bold = cell.modifier.contains(Modifier::BOLD) && self.bold.is_none();
            for (gx, gy, img) in &r.parts {
                blit_glyph(
                    frame,
                    stride,
                    width,
                    height,
                    px0 as i32 + gx,
                    py0 as i32 + gy,
                    img,
                    fg,
                );
                if synth_bold && !img.color {
                    blit_glyph(
                        frame,
                        stride,
                        width,
                        height,
                        px0 as i32 + gx + 1,
                        py0 as i32 + gy,
                        img,
                        fg,
                    );
                }
            }
        }
        if cell.modifier.contains(Modifier::UNDERLINED) {
            let uy = py0 + (self.baseline as u32 + 2).min(ch - 1);
            fill_rect(frame, stride, width, height, px0, uy, cw * wcells, 1, fg);
        }
        if cell.modifier.contains(Modifier::CROSSED_OUT) {
            let sy = py0 + ch * 55 / 100;
            fill_rect(frame, stride, width, height, px0, sy, cw * wcells, 1, fg);
        }
        wcells
    }

    fn cell_colors(&self, cell: &Cell) -> ([u8; 3], [u8; 3]) {
        (self.rgb(cell.fg, true), self.rgb(cell.bg, false))
    }

    fn blit_placement(
        &self,
        frame: &mut [u32],
        width: u32,
        height: u32,
        stride: u32,
        p: &Placement,
    ) {
        let bx = p.area.x as u32 * self.cell_w;
        let by = p.area.y as u32 * self.cell_h;
        let bw = p.area.width as u32 * self.cell_w;
        let bh = p.area.height as u32 * self.cell_h;
        if bw == 0 || bh == 0 {
            return;
        }
        let (iw, ih) = (p.image.width(), p.image.height());
        if iw == 0 || ih == 0 {
            return;
        }
        let scale = (bw as f32 / iw as f32).min(bh as f32 / ih as f32);
        let nw = ((iw as f32 * scale).round() as u32).clamp(1, bw);
        let nh = ((ih as f32 * scale).round() as u32).clamp(1, bh);
        let scaled;
        let img: &RgbaImage = if (nw, nh) == (iw, ih) {
            &p.image
        } else {
            scaled =
                image::imageops::resize(&*p.image, nw, nh, image::imageops::FilterType::Triangle);
            &scaled
        };
        let ox = bx + (bw - nw) / 2;
        let oy = by + (bh - nh) / 2;
        for yy in 0..nh {
            let ty = oy + yy;
            if ty >= height {
                break;
            }
            for xx in 0..nw {
                let tx = ox + xx;
                if tx >= width {
                    break;
                }
                let px = img.get_pixel(xx, yy).0;
                let idx = (ty * stride + tx) as usize;
                frame[idx] = blend(frame[idx], [px[0], px[1], px[2]], px[3]);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum FontChoice {
    Text,
    Bold,
    Emoji,
}

fn is_modifier_like(ch: char) -> bool {
    let cp = ch as u32;
    (0x1F3FB..=0x1F3FF).contains(&cp) || (0xE0020..=0xE007F).contains(&cp) || cp == 0x20E3
}

fn indexed_rgb(i: u8) -> [u8; 3] {
    match i {
        0..=15 => PALETTE16[i as usize],
        16..=231 => {
            let n = i - 16;
            let step = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            [step(n / 36), step((n / 6) % 6), step(n % 6)]
        }
        _ => {
            let v = 8 + (i - 232) * 10;
            [v, v, v]
        }
    }
}

#[inline]
fn pack(c: [u8; 3]) -> u32 {
    ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | c[2] as u32
}

#[inline]
fn unpack(p: u32) -> [u8; 3] {
    [
        ((p >> 16) & 0xff) as u8,
        ((p >> 8) & 0xff) as u8,
        (p & 0xff) as u8,
    ]
}

fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let f = |x: u8, y: u8| (x as f32 * (1.0 - t) + y as f32 * t).round() as u8;
    [f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2])]
}

#[inline]
fn blend(dst: u32, src: [u8; 3], alpha: u8) -> u32 {
    if alpha == 255 {
        return pack(src);
    }
    if alpha == 0 {
        return dst;
    }
    let d = unpack(dst);
    let a = alpha as u32;
    let f = |s: u8, d: u8| ((s as u32 * a + d as u32 * (255 - a)) / 255) as u8;
    pack([f(src[0], d[0]), f(src[1], d[1]), f(src[2], d[2])])
}

#[allow(clippy::too_many_arguments)] // the frame, its geometry and the rectangle
fn fill_rect(
    frame: &mut [u32],
    stride: u32,
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    c: [u8; 3],
) {
    let px = pack(c);
    let x1 = (x + w).min(width);
    let y1 = (y + h).min(height);
    for yy in y..y1 {
        let start = (yy * stride + x) as usize;
        let end = (yy * stride + x1) as usize;
        if start < end {
            frame[start..end].fill(px);
        }
    }
}

#[allow(clippy::too_many_arguments)] // the frame, its geometry and the glyph
fn blit_glyph(
    frame: &mut [u32],
    stride: u32,
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    img: &GlyphImage,
    fg: [u8; 3],
) {
    for yy in 0..img.height {
        let ty = y + yy as i32;
        if ty < 0 || ty as u32 >= height {
            continue;
        }
        for xx in 0..img.width {
            let tx = x + xx as i32;
            if tx < 0 || tx as u32 >= width {
                continue;
            }
            let idx = (ty as u32 * stride + tx as u32) as usize;
            if img.color {
                let i = ((yy * img.width + xx) * 4) as usize;
                let (r, g, b, a) = (
                    img.data[i],
                    img.data[i + 1],
                    img.data[i + 2],
                    img.data[i + 3],
                );
                frame[idx] = blend(frame[idx], [r, g, b], a);
            } else {
                let a = img.data[(yy * img.width + xx) as usize];
                frame[idx] = blend(frame[idx], fg, a);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, Borders, Paragraph, Widget};

    /// Render a sample frame to target/console-sample.png for eyeballing.
    #[test]
    fn painting_only_changed_cells_matches_a_full_repaint() {
        let Ok(fonts) = super::super::fonts::resolve(&Default::default()) else {
            eprintln!("no fonts via fc-match; skipping");
            return;
        };
        let mut r = Rasterizer::new(&fonts, 20.0).expect("rasterizer");
        let (cols, rows) = (24u16, 4u16);
        let (w, h) = (cols as u32 * r.cell_w, rows as u32 * r.cell_h);
        let area = Rect::new(0, 0, cols, rows);
        let mut buf = Buffer::empty(area);
        buf.set_string(
            1,
            1,
            "hello wide 漢字 text",
            Style::default().fg(Color::Cyan),
        );
        buf.set_string(
            1,
            2,
            "second row",
            Style::default().add_modifier(Modifier::BOLD),
        );
        let picture = Placement {
            area: Rect::new(14, 1, 4, 2),
            image: Arc::new(RgbaImage::from_pixel(
                4 * r.cell_w,
                2 * r.cell_h,
                image::Rgba([10, 200, 10, 255]),
            )),
        };
        let mut frame = vec![0u32; (w * h) as usize];
        r.render(
            &buf,
            std::slice::from_ref(&picture),
            Some((3, 2)),
            &mut frame,
            w,
            h,
            w,
        );

        // the text changes, the picture moves, the cursor moves
        let before = buf.clone();
        buf.set_string(
            1,
            1,
            "HELLO wide 漢字 text",
            Style::default().fg(Color::Red),
        );
        buf.set_string(1, 2, "          ", Style::default());
        let moved = Placement {
            area: Rect::new(15, 2, 4, 2),
            image: picture.image.clone(),
        };
        // the cells that changed, as the backend marks them from ratatui's diff
        let mut dirty: Vec<bool> = before
            .content
            .iter()
            .zip(&buf.content)
            .map(|(a, b)| a != b)
            .collect();
        for rect in [
            picture.area,
            moved.area,
            Rect::new(3, 2, 1, 1),
            Rect::new(5, 3, 1, 1),
        ] {
            for y in rect.y..rect.bottom() {
                for x in rect.x..rect.right() {
                    dirty[y as usize * cols as usize + x as usize] = true;
                }
            }
        }
        r.render_cells(
            &buf,
            &dirty,
            std::slice::from_ref(&moved),
            Some((5, 3)),
            &mut frame,
            w,
            h,
            w,
        );

        let mut full = vec![0u32; (w * h) as usize];
        r.render(
            &buf,
            std::slice::from_ref(&moved),
            Some((5, 3)),
            &mut full,
            w,
            h,
            w,
        );
        let mut bad: Vec<(u32, u32)> = Vec::new();
        for (i, (a, b)) in frame.iter().zip(&full).enumerate() {
            if a != b {
                let (px, py) = (i as u32 % w, i as u32 / w);
                bad.push((px / r.cell_w, py / r.cell_h));
            }
        }
        bad.sort();
        bad.dedup();
        assert!(
            bad.is_empty(),
            "cells that differ from a full repaint: {bad:?}"
        );
    }

    #[test]
    fn render_sample_frame() {
        let Ok(fonts) = super::super::fonts::resolve(&Default::default()) else {
            eprintln!("no fonts via fc-match; skipping");
            return;
        };
        let mut r = Rasterizer::new(&fonts, 28.0).expect("rasterizer");
        let (cols, rows) = (60u16, 8u16);
        let (w, h) = (cols as u32 * r.cell_w, rows as u32 * r.cell_h);
        let area = Rect::new(0, 0, cols, rows);
        let mut buf = Buffer::empty(area);
        let block = Block::default().borders(Borders::ALL).title(" Messages ");
        let inner = block.inner(area);
        block.render(area, &mut buf);
        let lines = vec![
            Line::from(vec![
                Span::styled(
                    "mock",
                    Style::default()
                        .fg(Color::LightCyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" hello 👀 🙂 👍🏽 🇺🇸 👨‍👩‍👧 end"),
            ]),
            Line::from(vec![
                Span::styled(
                    "dim timestamp",
                    Style::default().add_modifier(Modifier::DIM),
                ),
                Span::raw("  "),
                Span::styled("`code`", Style::default().fg(Color::Cyan)),
                Span::raw("  "),
                Span::styled(
                    "link",
                    Style::default()
                        .fg(Color::Blue)
                        .add_modifier(Modifier::UNDERLINED),
                ),
                Span::raw("  "),
                Span::styled(
                    " pill ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Rgb(88, 101, 242)),
                ),
                Span::raw("  "),
                Span::styled(
                    "selected",
                    Style::default().add_modifier(Modifier::REVERSED),
                ),
            ]),
            Line::from("ÄÖÜ ñ € → ─│┌┐ ⠀⠀ picture there"),
        ];
        Paragraph::new(lines).render(inner, &mut buf);
        // a picture over the braille placeholder (2 cells) and a bigger one
        let mut pic = RgbaImage::new(16, 16);
        for (x, y, p) in pic.enumerate_pixels_mut() {
            *p = image::Rgba([x as u8 * 16, y as u8 * 16, 128, 255]);
        }
        let placements = vec![
            Placement {
                area: Rect::new(inner.x + 20, inner.y + 2, 2, 1),
                image: Arc::new(pic.clone()),
            },
            Placement {
                area: Rect::new(inner.x + 40, inner.y + 3, 12, 4),
                image: Arc::new(pic),
            },
        ];
        let mut frame = vec![0u32; (w * h) as usize];
        r.render(
            &buf,
            &placements,
            Some((inner.x + 5, inner.y + 4)),
            &mut frame,
            w,
            h,
            w,
        );
        let mut out = image::RgbImage::new(w, h);
        for (i, p) in frame.iter().enumerate() {
            let c = unpack(*p);
            out.put_pixel((i as u32) % w, (i as u32) / w, image::Rgb(c));
        }
        std::fs::create_dir_all("target").ok();
        out.save("target/console-sample.png").expect("save png");
        eprintln!(
            "cell {}x{} frame {}x{} fonts {:?}",
            r.cell_w, r.cell_h, w, h, fonts
        );
    }
}
