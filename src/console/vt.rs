//! Owning the virtual terminal while we draw: graphics mode (so fbcon stops
//! painting text over us) and cooperative VT switching via signals.

use anyhow::{Context, Result};
use std::sync::atomic::{AtomicBool, Ordering};

const KDSETMODE: libc::c_ulong = 0x4B3A;
const KDGETMODE: libc::c_ulong = 0x4B3B;
const KD_TEXT: libc::c_int = 0;
const KD_GRAPHICS: libc::c_int = 1;
const VT_SETMODE: libc::c_ulong = 0x5602;
const VT_RELDISP: libc::c_ulong = 0x5605;
const VT_AUTO: libc::c_char = 0;
const VT_PROCESS: libc::c_char = 1;
const VT_ACKACQ: libc::c_int = 2;

#[repr(C)]
struct VtMode {
    mode: libc::c_char,
    waitv: libc::c_char,
    relsig: libc::c_short,
    acqsig: libc::c_short,
    frsig: libc::c_short,
}

static RELEASE_REQUESTED: AtomicBool = AtomicBool::new(false);
static ACQUIRE_REQUESTED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_release(_: libc::c_int) {
    RELEASE_REQUESTED.store(true, Ordering::SeqCst);
}
extern "C" fn on_acquire(_: libc::c_int) {
    ACQUIRE_REQUESTED.store(true, Ordering::SeqCst);
}

/// True when stdin is a Linux virtual console.
pub fn stdin_is_vt() -> bool {
    let mut mode: libc::c_int = 0;
    // SAFETY: KDGETMODE writes one int.
    unsafe { libc::ioctl(0, KDGETMODE as _, &mut mode as *mut _) == 0 }
}

pub struct VtGuard {
    fd: libc::c_int,
}

impl VtGuard {
    /// Switch the VT to graphics mode and ask the kernel to tell us about
    /// VT switches instead of doing them behind our back.
    pub fn take() -> Result<Self> {
        let fd = 0;
        // SAFETY: plain ioctls on stdin.
        unsafe {
            if libc::ioctl(fd, KDSETMODE as _, KD_GRAPHICS) != 0 {
                return Err(std::io::Error::last_os_error()).context("KDSETMODE KD_GRAPHICS");
            }
            libc::signal(libc::SIGUSR1, on_release as *const () as libc::sighandler_t);
            libc::signal(libc::SIGUSR2, on_acquire as *const () as libc::sighandler_t);
            let vm = VtMode {
                mode: VT_PROCESS,
                waitv: 0,
                relsig: libc::SIGUSR1 as libc::c_short,
                acqsig: libc::SIGUSR2 as libc::c_short,
                frsig: 0,
            };
            if libc::ioctl(fd, VT_SETMODE as _, &vm as *const _) != 0 {
                libc::ioctl(fd, KDSETMODE as _, KD_TEXT);
                return Err(std::io::Error::last_os_error()).context("VT_SETMODE");
            }
        }
        Ok(Self { fd })
    }

    /// The kernel asked us to let go of the VT (someone pressed Alt+Fn).
    pub fn take_release_request() -> bool {
        RELEASE_REQUESTED.swap(false, Ordering::SeqCst)
    }

    /// The VT is ours again.
    pub fn take_acquire_request() -> bool {
        ACQUIRE_REQUESTED.swap(false, Ordering::SeqCst)
    }

    pub fn ack_release(&self) {
        // SAFETY: plain ioctl.
        unsafe {
            libc::ioctl(self.fd, VT_RELDISP as _, 1);
        }
    }

    pub fn ack_acquire(&self) {
        // SAFETY: plain ioctl.
        unsafe {
            libc::ioctl(self.fd, VT_RELDISP as _, VT_ACKACQ);
            libc::ioctl(self.fd, KDSETMODE as _, KD_GRAPHICS);
        }
    }
}

impl Drop for VtGuard {
    fn drop(&mut self) {
        // SAFETY: restore text mode and automatic switching.
        unsafe {
            let vm = VtMode {
                mode: VT_AUTO,
                waitv: 0,
                relsig: 0,
                acqsig: 0,
                frsig: 0,
            };
            libc::ioctl(self.fd, VT_SETMODE as _, &vm as *const _);
            libc::ioctl(self.fd, KDSETMODE as _, KD_TEXT);
            libc::signal(libc::SIGUSR1, libc::SIG_DFL);
            libc::signal(libc::SIGUSR2, libc::SIG_DFL);
        }
    }
}

/// Best-effort text mode restore for a panic hook: a stuck graphics-mode
/// VT is the worst outcome of a crash.
pub fn emergency_restore() {
    // SAFETY: plain ioctls on stdin.
    unsafe {
        let vm = VtMode {
            mode: VT_AUTO,
            waitv: 0,
            relsig: 0,
            acqsig: 0,
            frsig: 0,
        };
        libc::ioctl(0, VT_SETMODE as _, &vm as *const _);
        libc::ioctl(0, KDSETMODE as _, KD_TEXT);
    }
}
