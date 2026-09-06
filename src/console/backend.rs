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

pub type SharedPlacements = Rc<RefCell<Vec<Placement>>>;

/// Per frame: the escape sequences of the pictures on screen, by the cell
/// they are printed at.
pub type SharedPictures = Rc<RefCell<HashMap<(u16, u16), Arc<str>>>>;

/// The crossterm backend, with pictures. ratatui's diff hands over the
/// cells that changed; a cell with a picture sentinel is not written as
/// text but replaced by the picture's escape sequences, printed at that
/// position. Everything else goes to crossterm as usual.
pub struct TermBackend<W: Write> {
    inner: CrosstermBackend<W>,
    pictures: SharedPictures,
}

impl<W: Write> TermBackend<W> {
    pub fn new(writer: W, pictures: SharedPictures) -> Self {
        Self {
            inner: CrosstermBackend::new(writer),
            pictures,
        }
    }
}

impl<W: Write> Backend for TermBackend<W> {
    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let pictures = self.pictures.borrow();
        let mut batch: Vec<(u16, u16, &Cell)> = Vec::new();
        for (x, y, cell) in content {
            if !crate::app::is_picture_sentinel(cell.underline_color) {
                batch.push((x, y, cell));
                continue;
            }
            // crossterm's draw starts and ends with the cursor and colours
            // in a known state, so text can be sent in pieces around a picture
            self.inner.draw(batch.drain(..))?;
            if let Some(data) = pictures.get(&(x, y)) {
                queue!(self.inner, MoveTo(x, y), Print(&**data))?;
            }
        }
        self.inner.draw(batch.into_iter())
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
        self.inner.clear()
    }
    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        self.inner.clear_region(clear_type)
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

    #[test]
    fn sentinel_cells_print_their_picture_and_text_flows_around() {
        let pictures: SharedPictures = Rc::new(RefCell::new(HashMap::new()));
        pictures
            .borrow_mut()
            .insert((3, 1), Arc::from("\x1bPq#0;2;0;0;0#0~~$-\x1b\\"));
        let recorder = Recorder::default();
        let mut backend = TermBackend::new(recorder.clone(), pictures);
        let mut a = Cell::new("a");
        let mut sentinel = Cell::new("\u{2800}");
        sentinel.set_style(crate::app::picture_sentinel_style(Style::default(), 7, 0));
        let mut b = Cell::new("b");
        a.set_style(Style::default());
        b.set_style(Style::default());
        backend
            .draw(vec![(2u16, 1u16, &a), (3, 1, &sentinel), (4, 1, &b)].into_iter())
            .unwrap();
        Backend::flush(&mut backend).unwrap();
        let out = String::from_utf8(recorder.0.borrow().clone()).unwrap();
        let pic = out.find("\x1bPq").expect("the picture is printed");
        let a_at = out.find('a').unwrap();
        let b_at = out.rfind('b').unwrap();
        assert!(a_at < pic && pic < b_at, "{out:?}");
        assert!(
            out[..pic].ends_with("\x1b[2;4H"),
            "moved to the cell first: {out:?}"
        );
        assert!(
            !out.contains('\u{2800}'),
            "the sentinel itself is never written"
        );
    }
}
