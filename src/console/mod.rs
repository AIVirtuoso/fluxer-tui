//! Console mode: on a Linux VT there is no terminal emulator to draw pictures
//! for us, so fluxter paints its whole screen itself. The rasteriser turns
//! the ratatui cell buffer (plus picture placements) into pixels, the fonts
//! module finds a text and an emoji font, the DRM output puts the pixels on
//! the display, and the VT module keeps the kernel console out of the way.

pub mod backend;
pub mod drm;
pub mod fonts;
pub mod output;
pub mod raster;
pub mod vt;

use anyhow::{Context, Result};
use std::path::PathBuf;

/// How console mode was selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// Draw through DRM on this device.
    Drm(PathBuf),
    /// Draw into PNG files (testing): directory and pixel size.
    Dump(PathBuf, (u32, u32)),
    /// Use the terminal as usual.
    Terminal,
}

/// Decide whether to run in console mode: `FLUXER_TUI_CONSOLE` first
/// ("never" | "always" | "dump:<dir>[:WxH]"), then the config, then, for
/// "auto", whether stdin is a VT.
pub fn select(mode: crate::config::ConsoleMode, drm_device: &str) -> Selection {
    let device = PathBuf::from(if drm_device.is_empty() {
        "/dev/dri/card0"
    } else {
        drm_device
    });
    if let Ok(v) = std::env::var("FLUXER_TUI_CONSOLE") {
        let v = v.trim();
        if let Some(rest) = v.strip_prefix("dump:") {
            let (dir, size) = match rest.rsplit_once(':') {
                Some((d, wh)) if wh.contains('x') => {
                    let (w, h) = wh.split_once('x').unwrap();
                    (d, (w.parse().unwrap_or(1280), h.parse().unwrap_or(720)))
                }
                _ => (rest, (1280, 720)),
            };
            return Selection::Dump(PathBuf::from(dir), size);
        }
        match v {
            "never" | "0" | "off" => return Selection::Terminal,
            "always" | "1" | "on" => return Selection::Drm(device),
            _ => {}
        }
    }
    use crate::config::ConsoleMode::*;
    match mode {
        Never => Selection::Terminal,
        Always => Selection::Drm(device),
        Auto => {
            let term_linux = std::env::var("TERM").map(|t| t == "linux").unwrap_or(false);
            if term_linux && vt::stdin_is_vt() {
                Selection::Drm(device)
            } else {
                Selection::Terminal
            }
        }
    }
}

/// Everything console mode needs to keep alive while the app runs.
pub struct ConsoleSession {
    /// Dropped last: restores text mode.
    pub vt: Option<vt::VtGuard>,
}

pub fn build_backend(
    selection: &Selection,
    cfg: &crate::config::ConsoleSettings,
    placements: backend::SharedPlacements,
) -> Result<(backend::ConsoleBackend, ConsoleSession)> {
    let requested = fonts::FontPaths {
        text: cfg.font.as_ref().map(PathBuf::from),
        bold: cfg.bold_font.as_ref().map(PathBuf::from),
        emoji: cfg.emoji_font.as_ref().map(PathBuf::from),
    };
    let paths = fonts::resolve(&requested)?;
    let px = if cfg.font_px > 0.0 { cfg.font_px } else { 28.0 };
    let raster = raster::Rasterizer::new(&paths, px).context("loading console fonts")?;
    let (out, vt): (Box<dyn output::Output>, Option<vt::VtGuard>) = match selection {
        Selection::Drm(dev) => {
            let vt = vt::VtGuard::take().context("taking over the virtual terminal")?;
            match drm::DrmOutput::open(dev) {
                Ok(o) => (Box::new(o), Some(vt)),
                Err(e) => {
                    drop(vt);
                    return Err(e);
                }
            }
        }
        Selection::Dump(dir, size) => (Box::new(output::PngDump::new(dir.clone(), *size)?), None),
        Selection::Terminal => anyhow::bail!("not a console selection"),
    };
    Ok((
        backend::ConsoleBackend::new(raster, out, placements),
        ConsoleSession { vt },
    ))
}
