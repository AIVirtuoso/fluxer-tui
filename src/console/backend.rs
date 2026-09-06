//! A ratatui backend that keeps the cell buffer and paints it with the
//! rasteriser on every flush, plus an enum that lets the app hold either
//! this or the usual crossterm backend.

use super::output::Output;
use super::raster::{Placement, Rasterizer};
use ratatui::backend::{Backend, ClearType, CrosstermBackend, WindowSize};
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::{Position, Rect, Size};
use std::cell::RefCell;
use std::io::{self, Stdout};
use std::rc::Rc;

pub type SharedPlacements = Rc<RefCell<Vec<Placement>>>;

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
    Crossterm(CrosstermBackend<Stdout>),
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
