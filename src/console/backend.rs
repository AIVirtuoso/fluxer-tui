//! A ratatui backend that keeps the cell buffer and paints it with the
//! rasteriser on every flush, a crossterm backend that prints terminal
//! pictures at sentinel cells, and an enum that lets the app hold either.

use super::output::Output;
use super::raster::{Placement, Rasterizer};
use crossterm::cursor::MoveTo;
use crossterm::queue;
use crossterm::style::Print;
use ratatui::backend::{Backend, ClearType, CrosstermBackend, WindowSize};
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::{Position, Rect, Size};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{self, Stdout, Write};
use std::rc::Rc;
use std::sync::Arc;
use unicode_width::UnicodeWidthStr;

pub type SharedPlacements = Rc<RefCell<Vec<Placement>>>;

/// A picture to print at a sentinel cell, and the cells it covers.
#[derive(Clone, Debug)]
pub struct PicturePrint {
    pub data: Arc<str>,
    pub area: Rect,
}

/// Per frame: the pictures on screen, by the cell they are printed at.
pub type SharedPictures = Rc<RefCell<HashMap<(u16, u16), PicturePrint>>>;

/// The message pane scrolled: its rows moved up by `rows` (down when
/// negative). The terminal can do that shift itself, and the pictures in
/// those rows move with them instead of being sent again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionScroll {
    pub area: Rect,
    pub rows: i32,
}

/// What the app hands the backend with each frame: a copy of the cells it
/// rendered, and the scroll the pane did, if any.
#[derive(Default, Debug)]
pub struct FrameInfo {
    pub buffer: Option<Buffer>,
    pub scroll: Option<RegionScroll>,
}

pub type SharedFrame = Rc<RefCell<FrameInfo>>;

/// What the terminal shows, cell by cell, as far as the backend knows: what
/// it wrote last, shifted when the terminal scrolled for it. Cells under a
/// picture are `covered`: whatever is written there next goes out even if
/// the cell reads unchanged, since the picture's pixels sit on it.
struct Shadow {
    area: Rect,
    cells: Vec<Cell>,
    covered: Vec<bool>,
}

impl Shadow {
    fn new(area: Rect, covered: bool) -> Self {
        let n = area.area() as usize;
        Self {
            area,
            cells: vec![Cell::default(); n],
            covered: vec![covered; n],
        }
    }

    fn index(&self, x: u16, y: u16) -> Option<usize> {
        (x < self.area.width && y < self.area.height)
            .then(|| y as usize * self.area.width as usize + x as usize)
    }

    /// The rows of `region`, across the whole width, moved up by `rows`
    /// (down when negative); the rows that come into view are blank.
    fn scroll(&mut self, region: Rect, rows: i32) {
        let w = self.area.width as usize;
        let top = region.y.min(self.area.height) as usize;
        let bottom = region.bottom().min(self.area.height) as usize;
        let h = bottom.saturating_sub(top);
        let n = (rows.unsigned_abs() as usize).min(h);
        if w == 0 || h == 0 || n == 0 {
            return;
        }
        let blank = |cells: &mut [Cell], covered: &mut [bool], y: usize| {
            for x in 0..w {
                cells[y * w + x] = Cell::default();
                covered[y * w + x] = false;
            }
        };
        if rows > 0 {
            for y in top..bottom - n {
                let (from, to) = ((y + n) * w, y * w);
                for x in 0..w {
                    self.cells[to + x] = self.cells[from + x].clone();
                    self.covered[to + x] = self.covered[from + x];
                }
            }
            for y in bottom - n..bottom {
                blank(&mut self.cells, &mut self.covered, y);
            }
        } else {
            for y in (top + n..bottom).rev() {
                let (from, to) = ((y - n) * w, y * w);
                for x in 0..w {
                    self.cells[to + x] = self.cells[from + x].clone();
                    self.covered[to + x] = self.covered[from + x];
                }
            }
            for y in top..top + n {
                blank(&mut self.cells, &mut self.covered, y);
            }
        }
    }

    fn cover(&mut self, area: Rect) {
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                if let Some(i) = self.index(x, y) {
                    self.covered[i] = true;
                }
            }
        }
    }

    /// Everything above and below `region` is stale.
    fn cover_outside_rows(&mut self, region: Rect) {
        let w = self.area.width as usize;
        for y in 0..self.area.height as usize {
            if (y as u16) < region.y || (y as u16) >= region.bottom() {
                self.covered[y * w..(y + 1) * w].fill(true);
            }
        }
    }
}

/// The crossterm backend, with pictures and its own idea of what the
/// terminal shows. Each frame it gets the whole rendered buffer from the
/// app and writes the cells that differ from its shadow of the terminal, so
/// a cell is sent only when the terminal really shows something else. A
/// cell with a picture sentinel is not written as text: the picture's
/// escape sequences are printed at that position instead. When the message
/// pane merely scrolled, the terminal is told to shift those rows itself,
/// which moves the pictures in them too, and only what came into view is
/// sent.
pub struct TermBackend<W: Write> {
    inner: CrosstermBackend<W>,
    pictures: SharedPictures,
    frame: SharedFrame,
    shadow: Shadow,
}

impl<W: Write> TermBackend<W> {
    pub fn new(writer: W, pictures: SharedPictures, frame: SharedFrame) -> Self {
        Self {
            inner: CrosstermBackend::new(writer),
            pictures,
            frame,
            shadow: Shadow::new(Rect::default(), true),
        }
    }

    fn reconcile(&mut self, buf: &Buffer, scroll: Option<RegionScroll>) -> io::Result<()> {
        if self.shadow.area != buf.area {
            // a terminal of a new size: nothing is known about what it shows
            self.shadow = Shadow::new(buf.area, true);
        }
        if let Some(s) = scroll
            && s.rows != 0
            && s.area.height > 0
            && (s.rows.unsigned_abs() as u16) < s.area.height
            && s.area.bottom() <= buf.area.height
        {
            // DECSTBM takes 1-based inclusive rows and scrolls the whole width
            let (top, bottom) = (s.area.y + 1, s.area.bottom());
            let seq = if s.rows > 0 {
                format!("\x1b[{top};{bottom}r\x1b[{}S\x1b[r", s.rows)
            } else {
                format!("\x1b[{top};{bottom}r\x1b[{}T\x1b[r", -s.rows)
            };
            queue!(self.inner, Print(seq))?;
            self.shadow.scroll(s.area, s.rows);
            // foot moves every sixel on the screen by the scrolled rows, not
            // only those in the region, and does not cut one that crosses
            // the region's edge: the rows outside are rewritten, which
            // erases whatever pixels landed there
            self.shadow.cover_outside_rows(s.area);
        }
        let pictures = self.pictures.borrow();
        let mut batch: Vec<(u16, u16, &Cell)> = Vec::new();
        // wide characters, as in ratatui's own diff: the cells a wide
        // character spills into are never written, and when it shrinks the
        // cells after it must be
        let mut invalidated = 0usize;
        let mut to_skip = 0usize;
        for (i, current) in buf.content.iter().enumerate() {
            let prev_width = self.shadow.cells[i].symbol().width();
            let changed = self.shadow.covered[i] || *current != self.shadow.cells[i];
            if !current.skip && to_skip == 0 && (changed || invalidated > 0) {
                let (x, y) = buf.pos_of(i);
                if crate::app::is_picture_sentinel(current.underline_color) {
                    self.inner.draw(batch.drain(..))?;
                    if let Some(p) = pictures.get(&(x, y)) {
                        queue!(self.inner, MoveTo(x, y), Print(&*p.data))?;
                        self.shadow.cover(p.area);
                    }
                } else {
                    batch.push((x, y, current));
                }
                self.shadow.cells[i] = current.clone();
                self.shadow.covered[i] = false;
            } else if !current.skip && to_skip > 0 {
                // the tail of a wide character: shown by the terminal, never written
                self.shadow.cells[i] = current.clone();
                self.shadow.covered[i] = false;
            }
            let width = current.symbol().width();
            to_skip = width.saturating_sub(1);
            invalidated = width.max(prev_width).max(invalidated).saturating_sub(1);
        }
        self.inner.draw(batch.into_iter())
    }
}

impl<W: Write> Backend for TermBackend<W> {
    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let (buffer, scroll) = {
            let mut frame = self.frame.borrow_mut();
            (frame.buffer.take(), frame.scroll.take())
        };
        match buffer {
            Some(buf) => self.reconcile(&buf, scroll),
            None => {
                // drawn without a frame from the app: pass it on, and treat
                // the terminal as unknown from here
                self.shadow = Shadow::new(self.shadow.area, true);
                self.inner.draw(content)
            }
        }
    }
    fn hide_cursor(&mut self) -> io::Result<()> {
        self.inner.hide_cursor()
    }
    fn show_cursor(&mut self) -> io::Result<()> {
        self.inner.show_cursor()
    }
    fn get_cursor_position(&mut self) -> io::Result<Position> {
        self.inner.get_cursor_position()
    }
    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.inner.set_cursor_position(position)
    }
    fn clear(&mut self) -> io::Result<()> {
        self.inner.clear()?;
        self.shadow = Shadow::new(self.shadow.area, false);
        Ok(())
    }
    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        self.inner.clear_region(clear_type)?;
        self.shadow = Shadow::new(self.shadow.area, true);
        Ok(())
    }
    fn size(&self) -> io::Result<Size> {
        self.inner.size()
    }
    fn window_size(&mut self) -> io::Result<WindowSize> {
        self.inner.window_size()
    }
    fn flush(&mut self) -> io::Result<()> {
        Backend::flush(&mut self.inner)
    }
}

pub struct ConsoleBackend {
    raster: Rasterizer,
    out: Box<dyn Output>,
    buf: Buffer,
    frame: Vec<u32>,
    cursor: Position,
    cursor_visible: bool,
    placements: SharedPlacements,
    cols: u16,
    rows: u16,
    width: u32,
    height: u32,
}

impl ConsoleBackend {
    pub fn new(raster: Rasterizer, out: Box<dyn Output>, placements: SharedPlacements) -> Self {
        let (width, height) = out.size();
        let (cols, rows) = raster.grid(width, height);
        Self {
            raster,
            out,
            buf: Buffer::empty(Rect::new(0, 0, cols, rows)),
            frame: vec![0; (width * height) as usize],
            cursor: Position::default(),
            cursor_visible: false,
            placements,
            cols,
            rows,
            width,
            height,
        }
    }

    /// Pixel size of one cell, for sizing pictures.
    pub fn cell_size(&self) -> (u32, u32) {
        (self.raster.cell_w, self.raster.cell_h)
    }

    /// VT switch handling, called from the main loop.
    pub fn suspend(&mut self) -> io::Result<()> {
        self.out.suspend()
    }

    pub fn resume(&mut self) -> io::Result<()> {
        self.repaint();
        self.out.resume(&self.frame)
    }

    fn repaint(&mut self) {
        let placements = self.placements.borrow();
        let cursor = self
            .cursor_visible
            .then_some((self.cursor.x, self.cursor.y));
        self.raster.render(
            &self.buf,
            &placements,
            cursor,
            &mut self.frame,
            self.width,
            self.height,
            self.width,
        );
    }
}

impl Backend for ConsoleBackend {
    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        for (x, y, cell) in content {
            if x < self.cols && y < self.rows {
                self.buf[(x, y)] = cell.clone();
            }
        }
        Ok(())
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.cursor_visible = false;
        Ok(())
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.cursor_visible = true;
        Ok(())
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        Ok(self.cursor)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.cursor = position.into();
        Ok(())
    }

    fn clear(&mut self) -> io::Result<()> {
        self.buf.reset();
        Ok(())
    }

    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        match clear_type {
            ClearType::All => self.clear(),
            _ => Ok(()),
        }
    }

    fn size(&self) -> io::Result<Size> {
        Ok(Size::new(self.cols, self.rows))
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        Ok(WindowSize {
            columns_rows: Size::new(self.cols, self.rows),
            pixels: Size::new(
                self.width.min(u16::MAX as u32) as u16,
                self.height.min(u16::MAX as u32) as u16,
            ),
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        self.repaint();
        self.out.present(&self.frame)
    }
}

/// The terminal backend the app runs on.
pub enum AnyBackend {
    Crossterm(TermBackend<Stdout>),
    Console(ConsoleBackend),
}

macro_rules! delegate {
    ($self:ident, $m:ident $(, $a:expr)*) => {
        match $self {
            AnyBackend::Crossterm(b) => b.$m($($a),*),
            AnyBackend::Console(b) => b.$m($($a),*),
        }
    };
}

impl Backend for AnyBackend {
    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        delegate!(self, draw, content)
    }
    fn hide_cursor(&mut self) -> io::Result<()> {
        delegate!(self, hide_cursor)
    }
    fn show_cursor(&mut self) -> io::Result<()> {
        delegate!(self, show_cursor)
    }
    fn get_cursor_position(&mut self) -> io::Result<Position> {
        delegate!(self, get_cursor_position)
    }
    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        let p: Position = position.into();
        delegate!(self, set_cursor_position, p)
    }
    fn clear(&mut self) -> io::Result<()> {
        delegate!(self, clear)
    }
    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        delegate!(self, clear_region, clear_type)
    }
    fn size(&self) -> io::Result<Size> {
        delegate!(self, size)
    }
    fn window_size(&mut self) -> io::Result<WindowSize> {
        delegate!(self, window_size)
    }
    fn flush(&mut self) -> io::Result<()> {
        delegate!(self, flush)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    /// A writer whose bytes can be read back after the backend owns it.
    #[derive(Clone, Default)]
    struct Recorder(Rc<RefCell<Vec<u8>>>);

    impl Write for Recorder {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct Rig {
        backend: TermBackend<Recorder>,
        recorder: Recorder,
        pictures: SharedPictures,
        frame: SharedFrame,
    }

    fn rig() -> Rig {
        let pictures: SharedPictures = Rc::new(RefCell::new(HashMap::new()));
        let frame: SharedFrame = Rc::new(RefCell::new(FrameInfo::default()));
        let recorder = Recorder::default();
        let backend = TermBackend::new(recorder.clone(), pictures.clone(), frame.clone());
        Rig {
            backend,
            recorder,
            pictures,
            frame,
        }
    }

    impl Rig {
        /// Draw a frame from text rows; what the backend wrote comes back.
        fn draw(&mut self, rows: &[&str], scroll: Option<RegionScroll>) -> String {
            self.draw_buf(buffer_of(rows), scroll)
        }

        fn draw_buf(&mut self, buf: Buffer, scroll: Option<RegionScroll>) -> String {
            self.recorder.0.borrow_mut().clear();
            *self.frame.borrow_mut() = FrameInfo {
                buffer: Some(buf),
                scroll,
            };
            self.backend.draw(std::iter::empty()).unwrap();
            Backend::flush(&mut self.backend).unwrap();
            String::from_utf8(self.recorder.0.borrow().clone()).unwrap()
        }
    }

    fn buffer_of(rows: &[&str]) -> Buffer {
        let w = rows.iter().map(|r| r.chars().count()).max().unwrap_or(0) as u16;
        let mut buf = Buffer::empty(Rect::new(0, 0, w, rows.len() as u16));
        for (y, row) in rows.iter().enumerate() {
            for (x, ch) in row.chars().enumerate() {
                buf[(x as u16, y as u16)].set_symbol(&ch.to_string());
            }
        }
        buf
    }

    #[test]
    fn only_changed_cells_are_written() {
        let mut r = rig();
        let first = r.draw(&["abcd", "efgh"], None);
        assert!(
            first.contains("abcd") && first.contains("efgh"),
            "{first:?}"
        );
        let second = r.draw(&["abcd", "efgh"], None);
        assert!(
            !second.contains('a') && !second.contains('h'),
            "nothing changed: {second:?}"
        );
        let third = r.draw(&["abXd", "efgh"], None);
        assert!(third.contains('X') && !third.contains('e'), "{third:?}");
    }

    #[test]
    fn a_scrolled_pane_is_shifted_by_the_terminal_and_only_new_rows_are_sent() {
        let mut r = rig();
        r.draw(
            &["status", "aaaaaa", "bbbbbb", "cccccc", "dddddd", "input!"],
            None,
        );
        let region = Rect::new(0, 1, 6, 4);
        let out = r.draw(
            &["status", "bbbbbb", "cccccc", "dddddd", "eeeeee", "input!"],
            Some(RegionScroll {
                area: region,
                rows: 1,
            }),
        );
        assert!(
            out.contains("\x1b[2;5r\x1b[1S\x1b[r"),
            "scroll rows 2..5 up by one: {out:?}"
        );
        assert!(
            out.contains("eeeeee"),
            "the row that came into view: {out:?}"
        );
        for moved in ["bbbbbb", "cccccc", "dddddd"] {
            assert!(
                !out.contains(moved),
                "{moved} moved with the scroll: {out:?}"
            );
        }
        // the rows outside the region are sent again: the terminal may have
        // dragged picture pixels over them
        assert!(out.contains("status") && out.contains("input!"), "{out:?}");
        // scrolling back down exposes a row at the top
        let out = r.draw(
            &["status", "aaaaaa", "bbbbbb", "cccccc", "dddddd", "input!"],
            Some(RegionScroll {
                area: region,
                rows: -1,
            }),
        );
        assert!(
            out.contains("\x1b[2;5r\x1b[1T\x1b[r") && out.contains("aaaaaa"),
            "{out:?}"
        );
        assert!(!out.contains("cccccc"), "{out:?}");
    }

    #[test]
    fn pictures_print_at_their_sentinel_move_with_the_scroll_and_cover_their_cells() {
        let mut r = rig();
        let area = Rect::new(1, 1, 2, 2);
        let picture = PicturePrint {
            data: Arc::from("\x1bPq#0;2;0;0;0#0~~$-\x1b\\"),
            area,
        };
        let sentinel = crate::app::picture_sentinel_style(Style::default(), 7, 0);
        let mut buf = buffer_of(&["....", "....", "....", "...."]);
        buf[(1, 1)].set_style(sentinel);
        for (x, y) in [(2, 1), (1, 2), (2, 2)] {
            buf[(x, y)].set_skip(true);
        }
        r.pictures.borrow_mut().insert((1, 1), picture.clone());
        let out = r.draw_buf(buf.clone(), None);
        assert_eq!(out.matches("\x1bPq").count(), 1, "printed once: {out:?}");
        assert!(out.contains("\x1b[2;2H\x1bPq"), "at its cell: {out:?}");

        // the same frame again: nothing is sent
        let out = r.draw_buf(buf.clone(), None);
        assert!(!out.contains("\x1bPq") && !out.contains('.'), "{out:?}");

        // the pane scrolls up by one: the picture moves with its rows, no reprint
        let mut moved = buffer_of(&["....", "....", "....", "...."]);
        moved[(1, 0)].set_style(sentinel);
        for (x, y) in [(2, 0), (1, 1), (2, 1)] {
            moved[(x, y)].set_skip(true);
        }
        r.pictures.borrow_mut().clear();
        r.pictures.borrow_mut().insert(
            (1, 0),
            PicturePrint {
                data: picture.data.clone(),
                area: Rect::new(1, 0, 2, 2),
            },
        );
        let out = r.draw_buf(
            moved,
            Some(RegionScroll {
                area: Rect::new(0, 0, 4, 4),
                rows: 1,
            }),
        );
        assert!(out.contains("\x1b[1;4r\x1b[1S\x1b[r"), "{out:?}");
        assert!(!out.contains("\x1bPq"), "not sent again: {out:?}");

        // the picture goes away: the cells it covered are written even
        // though they read the same as before it came
        r.pictures.borrow_mut().clear();
        let out = r.draw_buf(buffer_of(&["....", "....", "....", "...."]), None);
        assert!(
            out.contains("\x1b[1;2H") || out.contains("\x1b[1;1H"),
            "row 1 rewritten: {out:?}"
        );
        assert!(out.matches('.').count() >= 4, "the covered cells: {out:?}");
    }

    #[test]
    fn a_frame_without_the_buffer_falls_back_to_the_diff() {
        let mut r = rig();
        let mut a = Cell::new("a");
        a.set_style(Style::default());
        r.backend.draw(vec![(0u16, 0u16, &a)].into_iter()).unwrap();
        Backend::flush(&mut r.backend).unwrap();
        let out = String::from_utf8(r.recorder.0.borrow().clone()).unwrap();
        assert!(out.contains('a'), "{out:?}");
    }
}
