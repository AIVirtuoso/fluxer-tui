//! Where console frames go: the display through DRM, or PNG files when
//! testing without a VT.

use std::io;
use std::path::PathBuf;

pub trait Output {
    /// Pixel size of the frame this output wants.
    fn size(&self) -> (u32, u32);
    /// Show a frame: XRGB8888, one `u32` per pixel, stride == width.
    fn present(&mut self, frame: &[u32]) -> io::Result<()>;
    /// The VT is being switched away from us: stop touching the display.
    fn suspend(&mut self) -> io::Result<()> {
        Ok(())
    }
    /// The VT is ours again: take the display back and redraw.
    fn resume(&mut self, frame: &[u32]) -> io::Result<()> {
        self.present(frame)
    }
}

/// Test output: every frame becomes `frame-NNNN.ppm` (binary PPM, fast to
/// write) in a directory.
pub struct PngDump {
    dir: PathBuf,
    size: (u32, u32),
    n: u32,
}

impl PngDump {
    pub fn new(dir: PathBuf, size: (u32, u32)) -> io::Result<Self> {
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir, size, n: 0 })
    }
}

impl Output for PngDump {
    fn size(&self) -> (u32, u32) {
        self.size
    }

    fn present(&mut self, frame: &[u32]) -> io::Result<()> {
        let (w, h) = self.size;
        let mut bytes = Vec::with_capacity(20 + (w * h * 3) as usize);
        bytes.extend_from_slice(format!("P6\n{w} {h}\n255\n").as_bytes());
        for p in frame.iter().take((w * h) as usize) {
            bytes.push(((p >> 16) & 0xff) as u8);
            bytes.push(((p >> 8) & 0xff) as u8);
            bytes.push((p & 0xff) as u8);
        }
        self.n += 1;
        let path = self.dir.join(format!("frame-{:04}.ppm", self.n));
        let tmp = self.dir.join("frame.tmp");
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(tmp, path)
    }
}
