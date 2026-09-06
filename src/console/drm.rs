//! Display output through DRM/KMS with dumb buffers: what mpv's drm video
//! output does, minus the video. Works on any KMS driver, never touches the
//! legacy framebuffer emulation.

use super::output::Output;
use anyhow::{Context, Result, bail};
use drm::Device;
use drm::buffer::{Buffer as _, DrmFourcc};
use drm::control::{
    Device as ControlDevice, PageFlipFlags, connector, crtc, dumbbuffer::DumbBuffer, framebuffer,
};
use std::io;
use std::os::fd::AsFd;
use std::path::Path;

struct Card(std::fs::File);

impl AsFd for Card {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.0.as_fd()
    }
}
impl Device for Card {}
impl ControlDevice for Card {}

struct Fb {
    dumb: DumbBuffer,
    fb: framebuffer::Handle,
    pitch_px: u32,
}

pub struct DrmOutput {
    card: Card,
    crtc: crtc::Handle,
    connector: connector::Handle,
    mode: drm::control::Mode,
    size: (u32, u32),
    fbs: [Fb; 2],
    back: usize,
    active: bool,
    flip_pending: bool,
}

impl DrmOutput {
    pub fn open(path: &Path) -> Result<Self> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .with_context(|| format!("opening {}", path.display()))?;
        let card = Card(file);
        // Master is needed for modesetting. On a VT that nobody else owns
        // (a plain getty) the first opener gets it; a compositor or kmscon
        // on another VT does not hold it while switched away.
        card.acquire_master_lock()
            .context("becoming DRM master (is another display server active on this VT?)")?;

        let res = card.resource_handles().context("DRM resources")?;
        let mut chosen = None;
        for handle in res.connectors() {
            let info = match card.get_connector(*handle, true) {
                Ok(i) => i,
                Err(_) => continue,
            };
            if info.state() != connector::State::Connected || info.modes().is_empty() {
                continue;
            }
            // prefer the CRTC/mode the connector is already driving
            let mut crtc_mode = None;
            if let Some(enc) = info.current_encoder()
                && let Ok(enc) = card.get_encoder(enc)
                && let Some(c) = enc.crtc()
                && let Ok(ci) = card.get_crtc(c)
            {
                crtc_mode = Some((c, ci.mode()));
            }
            let (crtc_h, mode) = match crtc_mode {
                Some((c, Some(m))) => (c, m),
                Some((c, None)) => (c, info.modes()[0]),
                None => {
                    // any CRTC one of its encoders can use
                    let mut pick = None;
                    for enc in info.encoders() {
                        if let Ok(e) = card.get_encoder(*enc)
                            && let Some(c) = res.filter_crtcs(e.possible_crtcs()).first()
                        {
                            pick = Some(*c);
                            break;
                        }
                    }
                    match pick {
                        Some(c) => (c, info.modes()[0]),
                        None => continue,
                    }
                }
            };
            chosen = Some((info.handle(), crtc_h, mode));
            break;
        }
        let Some((connector, crtc, mode)) = chosen else {
            bail!("no connected display found on the DRM device");
        };
        let (w, h) = mode.size();
        let size = (w as u32, h as u32);

        let make_fb = |card: &Card| -> Result<Fb> {
            let dumb = card
                .create_dumb_buffer(size, DrmFourcc::Xrgb8888, 32)
                .context("creating dumb buffer")?;
            let fb = card
                .add_framebuffer(&dumb, 24, 32)
                .context("creating framebuffer")?;
            let pitch_px = dumb.pitch() / 4;
            Ok(Fb { dumb, fb, pitch_px })
        };
        let fbs = [make_fb(&card)?, make_fb(&card)?];

        card.set_crtc(crtc, Some(fbs[0].fb), (0, 0), &[connector], Some(mode))
            .context("setting the display mode")?;
        Ok(Self {
            card,
            crtc,
            connector,
            mode,
            size,
            fbs,
            back: 1,
            active: true,
            flip_pending: false,
        })
    }

    fn copy_into(&mut self, idx: usize, frame: &[u32]) -> io::Result<()> {
        let (w, h) = self.size;
        let pitch = self.fbs[idx].pitch_px;
        let mut map = self.card.map_dumb_buffer(&mut self.fbs[idx].dumb)?;
        let bytes: &mut [u8] = map.as_mut();
        // SAFETY: XRGB8888 is 4 bytes per pixel and the mapping is at least
        // pitch*height bytes; we write whole u32 pixels.
        let px: &mut [u32] = unsafe {
            std::slice::from_raw_parts_mut(bytes.as_mut_ptr() as *mut u32, bytes.len() / 4)
        };
        for y in 0..h as usize {
            let src = &frame[y * w as usize..(y + 1) * w as usize];
            let dst = &mut px[y * pitch as usize..y * pitch as usize + w as usize];
            dst.copy_from_slice(src);
        }
        Ok(())
    }

    /// Block until the pending flip landed, so the buffer we are about to
    /// draw into is no longer on screen.
    fn wait_flip(&mut self) {
        if !self.flip_pending {
            return;
        }
        let fd = self.card.as_fd();
        let mut pfd = libc::pollfd {
            fd: std::os::fd::AsRawFd::as_raw_fd(&fd),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: pollfd is fully initialised and lives for the call.
        let n = unsafe { libc::poll(&mut pfd, 1, 200) };
        if n > 0
            && let Ok(events) = self.card.receive_events()
        {
            for _ in events {}
        }
        self.flip_pending = false;
    }
}

impl Output for DrmOutput {
    fn size(&self) -> (u32, u32) {
        self.size
    }

    fn present(&mut self, frame: &[u32]) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        self.wait_flip();
        let idx = self.back;
        self.copy_into(idx, frame)?;
        match self
            .card
            .page_flip(self.crtc, self.fbs[idx].fb, PageFlipFlags::EVENT, None)
        {
            Ok(()) => self.flip_pending = true,
            // some drivers refuse flips right after a modeset; a full set works
            Err(_) => self
                .card
                .set_crtc(
                    self.crtc,
                    Some(self.fbs[idx].fb),
                    (0, 0),
                    &[self.connector],
                    Some(self.mode),
                )
                .map_err(io::Error::other)?,
        }
        self.back = 1 - idx;
        Ok(())
    }

    fn suspend(&mut self) -> io::Result<()> {
        self.wait_flip();
        self.active = false;
        let _ = self.card.release_master_lock();
        Ok(())
    }

    fn resume(&mut self, frame: &[u32]) -> io::Result<()> {
        self.card.acquire_master_lock()?;
        self.active = true;
        let idx = self.back;
        self.copy_into(idx, frame)?;
        self.card
            .set_crtc(
                self.crtc,
                Some(self.fbs[idx].fb),
                (0, 0),
                &[self.connector],
                Some(self.mode),
            )
            .map_err(io::Error::other)?;
        self.back = 1 - idx;
        Ok(())
    }
}

impl Drop for DrmOutput {
    fn drop(&mut self) {
        self.wait_flip();
        for fb in &mut self.fbs {
            let _ = self.card.destroy_framebuffer(fb.fb);
        }
        // dumb buffers are freed with the file; the kernel restores fbcon
        // when the master goes away
        let _ = self.card.release_master_lock();
    }
}
