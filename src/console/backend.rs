//! A ratatui backend that keeps the cell buffer and paints it with the
//! rasteriser on every flush, a crossterm backend that prints terminal
//! pictures at sentinel cells, and an enum that lets the app hold either.

use super::output::Output;
use super::raster::{Placement, Rasterizer};
use crossterm::cursor::MoveTo;
use crossterm::queue;
use crossterm::style::Print;
use crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};
use ratatui::backend::{Backend, ClearType, CrosstermBackend, WindowSize};
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::{Position, Rect, Size};
use ratatui::style::Color;
use std::borrow::Cow;
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
    /// Image data to send once before any row of this picture (kitty),
    /// keyed by the picture and frame it belongs to.
    pub transmit: Option<(u32, Arc<str>)>,
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
    /// (down when negative); the rows that come into view hold nothing.
    ///
    /// Nothing, that is, of what the terminal was told to write there. A
    /// terminal draws every picture on the screen shifted by the scrolled
    /// rows and cut to the region, and leaves the pixels of the one it
    /// copied wherever they fall outside it: so a picture below the region
    /// paints into the rows arriving at the bottom, and one above it into
    /// those arriving at the top, while the cells there read blank. What a
    /// cell may have caught is what the cell `rows` away from it carried,
    /// wherever that cell was, so the arriving rows take `covered` from
    /// beyond the region's edge rather than nothing at all.
    fn scroll(&mut self, region: Rect, rows: i32) {
        let w = self.area.width as usize;
        let height = self.area.height as usize;
        let top = region.y.min(self.area.height) as usize;
        let bottom = region.bottom().min(self.area.height) as usize;
        let h = bottom.saturating_sub(top);
        let n = (rows.unsigned_abs() as usize).min(h);
        if w == 0 || h == 0 || n == 0 {
            return;
        }
        // The row an arriving one reads its pixels from lies outside the
        // region, which the shift below never writes, so it still says
        // what it did before the scroll.
        let source = |y: usize| {
            if rows > 0 {
                (y + n < height).then_some(y + n)
            } else {
                y.checked_sub(n)
            }
        };
        let arriving = if rows > 0 {
            for y in top..bottom - n {
                let (from, to) = ((y + n) * w, y * w);
                for x in 0..w {
                    self.cells[to + x] = self.cells[from + x].clone();
                    self.covered[to + x] = self.covered[from + x];
                }
            }
            bottom - n..bottom
        } else {
            for y in (top + n..bottom).rev() {
                let (from, to) = ((y - n) * w, y * w);
                for x in 0..w {
                    self.cells[to + x] = self.cells[from + x].clone();
                    self.covered[to + x] = self.covered[from + x];
                }
            }
            top..top + n
        };
        for y in arriving {
            let from = source(y).map(|s| s * w);
            for x in 0..w {
                self.cells[y * w + x] = Cell::default();
                self.covered[y * w + x] = match from {
                    // The cell a picture is printed at is left uncovered,
                    // since covering it is what tells the next frame to
                    // blank the picture away first; its pixels are on the
                    // screen all the same, and they are dragged along with
                    // the rest.
                    Some(f) => {
                        self.covered[f + x]
                            || crate::app::is_picture_sentinel(self.cells[f + x].underline_color)
                    }
                    None => false,
                };
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
/// pane merely scrolled, a terminal that can hold a frame back is told to
/// shift those rows itself, which moves the pictures in them too, and only
/// what came into view is sent.
pub struct TermBackend<W: Write> {
    inner: CrosstermBackend<W>,
    pictures: SharedPictures,
    frame: SharedFrame,
    shadow: Shadow,
    /// Pictures whose image data the terminal has (kitty).
    transmitted: std::collections::HashSet<u32>,
    /// Whether the terminal holds a frame back until it is whole (DEC mode
    /// 2026). Only then is it asked to scroll the pane: see `reconcile`.
    /// Off until the terminal has said so, since the cost of asking a
    /// terminal that cannot is a flicker the reader sees.
    can_hold_frame: bool,
}

/// Write the cells collected so far and start a new batch.
fn draw_batch<W: Write>(
    inner: &mut CrosstermBackend<W>,
    batch: &mut Vec<(u16, u16, Cow<Cell>)>,
) -> io::Result<()> {
    inner.draw(batch.iter().map(|(x, y, c)| (*x, *y, &**c)))?;
    batch.clear();
    Ok(())
}

/// A cell as the terminal is allowed to see it. The marker underline
/// colours mean something only inside the buffer, and a terminal that does
/// not know SGR 58 reads the sequence crossterm sends for one as ordinary
/// attributes: see [`crate::app::is_marker_underline`]. Nothing on screen
/// depends on them, so they come off here, at the one place cells are
/// written, rather than at each of the panes that set them.
fn as_written(cell: &Cell) -> Cow<'_, Cell> {
    if crate::app::is_marker_underline(cell.underline_color) {
        let mut plain = cell.clone();
        plain.underline_color = Color::Reset;
        Cow::Owned(plain)
    } else {
        Cow::Borrowed(cell)
    }
}

impl<W: Write> TermBackend<W> {
    pub fn new(writer: W, pictures: SharedPictures, frame: SharedFrame) -> Self {
        Self {
            inner: CrosstermBackend::new(writer),
            pictures,
            frame,
            shadow: Shadow::new(Rect::default(), true),
            transmitted: std::collections::HashSet::new(),
            can_hold_frame: false,
        }
    }

    /// What the terminal answered about DEC mode 2026 at start.
    pub fn set_can_hold_frame(&mut self, yes: bool) {
        self.can_hold_frame = yes;
    }

    fn reconcile(&mut self, buf: &Buffer, scroll: Option<RegionScroll>) -> io::Result<()> {
        // The frame goes out as one synchronized update: a terminal that
        // scrolls its pictures along with the rows would otherwise show them
        // past the pane's edge for an instant, before the rows outside are
        // rewritten. The update ends in `flush`, after the cursor is placed.
        queue!(self.inner, BeginSynchronizedUpdate)?;
        if self.shadow.area != buf.area {
            // a terminal of a new size: nothing is known about what it shows
            self.shadow = Shadow::new(buf.area, true);
        }
        // The pane is scrolled by the terminal only where a frame is shown
        // whole. DECSTBM scrolls every column of those rows, and there is no
        // way to hold it to the pane's: left and right margins (DECLRMM,
        // private mode 69) are answered "never heard of it" by both foot and
        // xterm. So the servers and channels boxes move with the pane and
        // are written back in the same frame -- invisible under a
        // synchronized update, a flicker of both boxes on every scroll step
        // without one. Where the terminal cannot hold a frame back the pane
        // is simply redrawn instead, which touches nothing outside it.
        if let Some(s) = scroll.filter(|_| self.can_hold_frame)
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
        let mut batch: Vec<(u16, u16, Cow<Cell>)> = Vec::new();
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
                    draw_batch(&mut self.inner, &mut batch)?;
                    if let Some(p) = pictures.get(&(x, y)) {
                        // A picture paints everything any of its frames
                        // paints, so showing the next frame replaces the
                        // one before outright and nothing has to be
                        // blanked. Only a different picture arriving at
                        // these cells needs them cleared first, and
                        // blanking on every frame is what an animation
                        // blinks with on a terminal that cannot hold a
                        // frame back until it is whole.
                        let arrived = self.shadow.covered[i]
                            || crate::app::picture_serial(self.shadow.cells[i].underline_color)
                                != crate::app::picture_serial(current.underline_color);
                        if y == p.area.y && arrived {
                            // The cells under a picture are never written
                            // while it is there, and a picture need not
                            // cover them all: it is cut to whole sixel
                            // bands, and what it leaves transparent shows
                            // whatever was on the screen. Blank those first,
                            // so what shows is the background rather than
                            // the frame before; the rows the picture paints
                            // in full are left alone, since blanking them
                            // only puts a blink in front of the picture.
                            let blanks: Vec<(u16, u16, Cell)> = (p.area.y..p.area.bottom())
                                .flat_map(|yy| (p.area.x..p.area.right()).map(move |xx| (xx, yy)))
                                .filter_map(|(xx, yy)| {
                                    let cell = buf.cell((xx, yy))?;
                                    let mut blank = Cell::default();
                                    blank.set_fg(cell.fg).set_bg(cell.bg);
                                    Some((xx, yy, blank))
                                })
                                .collect();
                            self.inner
                                .draw(blanks.iter().map(|(bx, by, c)| (*bx, *by, c)))?;
                        }
                        if let Some((key, seq)) = &p.transmit
                            && self.transmitted.insert(*key)
                        {
                            queue!(self.inner, Print(&**seq))?;
                        }
                        queue!(self.inner, MoveTo(x, y), Print(&*p.data))?;
                        self.shadow.cover(p.area);
                    }
                } else {
                    batch.push((x, y, as_written(current)));
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
        draw_batch(&mut self.inner, &mut batch)
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
                queue!(self.inner, BeginSynchronizedUpdate)?;
                let mut batch: Vec<(u16, u16, Cow<Cell>)> =
                    content.map(|(x, y, c)| (x, y, as_written(c))).collect();
                draw_batch(&mut self.inner, &mut batch)
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
        self.transmitted.clear();
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
        queue!(self.inner, EndSynchronizedUpdate)?;
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
    /// Cells changed since the last repaint, and whether the whole frame
    /// has to be painted (first frame, after a clear or a VT switch).
    dirty: Vec<bool>,
    full: bool,
    /// Where pictures and the cursor were last frame: painted over when
    /// they move or go.
    last_placements: Vec<Rect>,
    last_cursor: Option<(u16, u16)>,
}

fn mark_rect(dirty: &mut [bool], cols: u16, rows: u16, rect: Rect) {
    for y in rect.y..rect.bottom().min(rows) {
        for x in rect.x..rect.right().min(cols) {
            dirty[y as usize * cols as usize + x as usize] = true;
        }
    }
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
            dirty: vec![false; cols as usize * rows as usize],
            full: true,
            last_placements: Vec::new(),
            last_cursor: None,
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
        self.full = true;
        self.repaint();
        self.out.resume(&self.frame)
    }

    /// Paint the frame: everything after a clear, otherwise only the cells
    /// that changed, plus where pictures and the cursor were and are.
    fn repaint(&mut self) {
        let placements = self.placements.borrow();
        let cursor = self
            .cursor_visible
            .then_some((self.cursor.x, self.cursor.y));
        if self.full {
            self.raster.render(
                &self.buf,
                &placements,
                cursor,
                &mut self.frame,
                self.width,
                self.height,
                self.width,
            );
            self.full = false;
        } else {
            for rect in self
                .last_placements
                .iter()
                .copied()
                .chain(placements.iter().map(|p| p.area))
            {
                mark_rect(&mut self.dirty, self.cols, self.rows, rect);
            }
            for (x, y) in [self.last_cursor, cursor].into_iter().flatten() {
                mark_rect(&mut self.dirty, self.cols, self.rows, Rect::new(x, y, 1, 1));
            }
            self.raster.render_cells(
                &self.buf,
                &self.dirty,
                &placements,
                cursor,
                &mut self.frame,
                self.width,
                self.height,
                self.width,
            );
        }
        self.dirty.fill(false);
        self.last_placements = placements.iter().map(|p| p.area).collect();
        self.last_cursor = cursor;
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
                self.dirty[y as usize * self.cols as usize + x as usize] = true;
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
        self.full = true;
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

impl AnyBackend {
    /// Pass on what the terminal said about DEC mode 2026. The console
    /// renderer paints whole frames of its own and has nothing to answer.
    pub fn set_can_hold_frame(&mut self, yes: bool) {
        if let AnyBackend::Crossterm(b) = self {
            b.set_can_hold_frame(yes);
        }
    }
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

    /// A terminal that holds a frame back until it is whole, which is the
    /// only one asked to scroll the pane. The other has its own test.
    fn rig() -> Rig {
        let pictures: SharedPictures = Rc::new(RefCell::new(HashMap::new()));
        let frame: SharedFrame = Rc::new(RefCell::new(FrameInfo::default()));
        let recorder = Recorder::default();
        let mut backend = TermBackend::new(recorder.clone(), pictures.clone(), frame.clone());
        backend.set_can_hold_frame(true);
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

    /// A picture paints everything any of its frames paints, so the next
    /// frame replaces the one before outright and the cells need no
    /// blanking. Blanking them on every frame is what an animation blinks
    /// with on a terminal that cannot hold a frame back. Only a different
    /// picture arriving at those cells needs them cleared first.
    #[test]
    fn only_a_different_picture_blanks_the_cells_first() {
        let sixel: Arc<str> = Arc::from("\x1bPq#0;2;0;0;0#0~~$-\x1b\\");
        let show = |r: &mut Rig, serial: u16, frame: usize| {
            let mut buf = buffer_of(&["....", "....", "....", "...."]);
            buf[(1, 1)].set_style(crate::app::picture_sentinel_style(
                Style::default(),
                serial,
                frame,
            ));
            for (x, y) in [(2, 1), (1, 2), (2, 2)] {
                buf[(x, y)].set_skip(true);
            }
            r.pictures.borrow_mut().clear();
            r.pictures.borrow_mut().insert(
                (1, 1),
                PicturePrint {
                    data: sixel.clone(),
                    area: Rect::new(1, 1, 2, 2),
                    transmit: None,
                },
            );
            r.draw_buf(buf, None)
        };
        let mut r = rig();
        // it arrives: the cells are cleared of whatever was there
        let out = show(&mut r, 7, 0);
        assert!(out.contains("\x1b[3;2H  "), "blanked on arrival: {out:?}");

        // the next frame of the same picture: printed, nothing blanked
        let out = show(&mut r, 7, 1);
        assert!(out.contains("\x1bPq"), "still printed: {out:?}");
        assert!(
            !out.contains("\x1b[3;2H  "),
            "no blanking between frames: {out:?}"
        );

        // a different picture at the same cells: cleared again
        let out = show(&mut r, 8, 0);
        assert!(
            out.contains("\x1b[3;2H  "),
            "blanked for the new one: {out:?}"
        );
    }

    #[test]
    fn a_frame_is_one_synchronized_update() {
        let mut r = rig();
        let out = r.draw(&["ab", "cd"], None);
        assert!(out.starts_with("\x1b[?2026h"), "{out:?}");
        assert!(out.ends_with("\x1b[?2026l"), "{out:?}");
        let out = r.draw(
            &["ab", "xd"],
            Some(RegionScroll {
                area: Rect::new(0, 0, 2, 2),
                rows: 1,
            }),
        );
        let begin = out.find("\x1b[?2026h").unwrap();
        let scroll = out.find("\x1b[1;2r").unwrap();
        let end = out.find("\x1b[?2026l").unwrap();
        assert!(
            begin < scroll && scroll < end,
            "the scroll is inside the update: {out:?}"
        );
    }

    /// DECSTBM scrolls every column of the rows it is given, so the boxes
    /// beside the pane are dragged along and have to be written back. Under
    /// a synchronized update that is part of the same frame and cannot be
    /// seen; without one the reader sees both boxes go and come back on
    /// every scroll step. So a terminal that cannot hold a frame back is
    /// never asked to scroll, and the pane is redrawn instead.
    #[test]
    fn the_boxes_beside_the_pane_are_left_alone_where_a_frame_is_not_held_back() {
        let before = [
            "S:.........",
            "news:aaaaaa",
            "chat:bbbbbb",
            "help:cccccc",
            "meme:dddddd",
            "in:........",
        ];
        // the same sidebar, the pane scrolled up by one row
        let after = [
            "S:.........",
            "news:bbbbbb",
            "chat:cccccc",
            "help:dddddd",
            "meme:eeeeee",
            "in:........",
        ];
        let scroll = Some(RegionScroll {
            area: Rect::new(5, 1, 6, 4),
            rows: 1,
        });

        let mut r = rig();
        r.backend.set_can_hold_frame(false);
        r.draw(&before, None);
        let out = r.draw(&after, scroll);
        assert!(
            !out.contains("\x1b[2;5r"),
            "the terminal is not asked to scroll: {out:?}"
        );
        for box_label in ["news", "chat", "help", "meme"] {
            assert!(
                !out.contains(box_label),
                "{box_label} was written again: {out:?}"
            );
        }
        assert!(
            out.contains("bbbbbb") && out.contains("eeeeee"),
            "the pane is redrawn where it moved: {out:?}"
        );

        // the terminal that can: it scrolls, and pays for it by writing the
        // boxes back inside the same frame
        let mut r = rig();
        r.draw(&before, None);
        let out = r.draw(&after, scroll);
        assert!(out.contains("\x1b[2;5r\x1b[1S\x1b[r"), "{out:?}");
        assert!(
            out.contains("news") && out.contains("meme"),
            "the boxes come back in the same frame: {out:?}"
        );
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
        // only the update markers and crossterm's colour resets go out
        assert!(
            !second.contains('a') && !second.contains('e'),
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
            transmit: None,
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
        // its cells are blanked first, so what a sixel leaves uncovered
        // shows the background rather than what was there
        let blank_second_row = out.find("\x1b[3;2H  ").expect("blanks on the second row");
        assert!(blank_second_row < out.find("\x1bPq").unwrap(), "{out:?}");

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
                transmit: None,
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

    /// A terminal draws every picture on the screen shifted by the rows a
    /// region scroll moved, cut to that region, and leaves the pixels of
    /// the one it copied wherever they fall outside it (measured in foot,
    /// whose sixels this is about). So a picture below the pane -- the
    /// compose box's thumbnail -- paints into the row arriving at the
    /// pane's bottom, and one above it into the row arriving at the top,
    /// while the cells there say they are blank. Those rows are written
    /// even when nothing about them changed, or a slice of the thumbnail
    /// rides up the pane on every scroll and shows through wherever no
    /// text is laid over it.
    #[test]
    fn the_rows_that_arrive_are_written_where_a_picture_lies_beyond_the_edge() {
        let sentinel = crate::app::picture_sentinel_style(Style::default(), 7, 0);
        let picture = |y: u16| PicturePrint {
            data: Arc::from("\x1bPq#0;2;0;0;0#0~~$-\x1b\\"),
            area: Rect::new(0, y, 6, 1),
            transmit: None,
        };
        let with_picture = |rows: &[&str], y: u16| {
            let mut buf = buffer_of(rows);
            buf[(0, y)].set_style(sentinel);
            for x in 1..6 {
                buf[(x, y)].set_skip(true);
            }
            buf
        };
        let region = Rect::new(0, 1, 6, 4);

        // a thumbnail in the compose box, under the pane
        let mut r = rig();
        r.pictures.borrow_mut().insert((0, 5), picture(5));
        let rows = ["status", "aaaaaa", "bbbbbb", "cccccc", "dddddd", "......"];
        r.draw_buf(with_picture(&rows, 5), None);
        // the pane scrolls up by one; the row arriving at its bottom is
        // blank, which is what the backend thinks is there already
        let rows = ["status", "bbbbbb", "cccccc", "dddddd", "      ", "......"];
        let out = r.draw_buf(
            with_picture(&rows, 5),
            Some(RegionScroll {
                area: region,
                rows: 1,
            }),
        );
        assert!(
            out.contains("\x1b[5;1H      "),
            "the arriving row is written over the pixels dragged into it: {out:?}"
        );

        // and the same the other way: a picture above the pane paints into
        // the row arriving at the top when the pane scrolls back down
        let mut r = rig();
        r.pictures.borrow_mut().insert((0, 0), picture(0));
        let rows = ["......", "bbbbbb", "cccccc", "dddddd", "eeeeee", "input!"];
        r.draw_buf(with_picture(&rows, 0), None);
        let rows = ["......", "      ", "bbbbbb", "cccccc", "dddddd", "input!"];
        let out = r.draw_buf(
            with_picture(&rows, 0),
            Some(RegionScroll {
                area: region,
                rows: -1,
            }),
        );
        assert!(
            out.contains("\x1b[2;1H      "),
            "the row arriving at the top is written too: {out:?}"
        );
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

    /// A marker's slot number rides in the underline colour, which crossterm
    /// sends as `ESC[58;2;r;g;b m`. A terminal without SGR 58 skips the 58
    /// and runs the rest as ordinary attributes, so slot 42 is a reset and a
    /// green background there, on that cell and on every one written after
    /// it until something writes them again. Nothing on screen wants a
    /// marker, so none is written; an underline colour that is not a marker
    /// still is.
    #[test]
    fn marker_underline_colours_are_never_written() {
        let mut r = rig();
        let mut buf = buffer_of(&["abcde"]);
        buf[(1, 0)].set_style(crate::app::media_marker_style(42, 1));
        buf[(2, 0)].set_style(crate::app::custom_emoji_marker_style(42));
        buf[(3, 0)].set_style(crate::app::picture_sentinel_style(Style::default(), 42, 0));
        let out = r.draw_buf(buf, None);
        assert!(!out.contains("58;"), "a marker was written: {out:?}");
        assert!(
            out.contains('a') && out.contains('c'),
            "cells still print: {out:?}"
        );

        let mut buf = buffer_of(&["vwxyz"]);
        buf[(1, 0)].set_style(Style::default().underline_color(Color::Rgb(1, 2, 3)));
        let out = r.draw_buf(buf, None);
        assert!(out.contains("58;2;1;2;3"), "a real colour is kept: {out:?}");
    }

    /// The same for a frame the app did not hand over, which is passed
    /// straight to the diff.
    #[test]
    fn a_marker_is_taken_off_the_fallback_path_too() {
        let mut r = rig();
        let mut a = Cell::new("a");
        a.set_style(crate::app::custom_emoji_marker_style(42));
        r.backend.draw(vec![(0u16, 0u16, &a)].into_iter()).unwrap();
        Backend::flush(&mut r.backend).unwrap();
        let out = String::from_utf8(r.recorder.0.borrow().clone()).unwrap();
        assert!(!out.contains("58;"), "a marker was written: {out:?}");
        assert!(out.contains('a'), "{out:?}");
    }
}
