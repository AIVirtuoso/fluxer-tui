//! The terminal's own background colour, asked for with OSC 11.
//!
//! Sixel and halfblocks carry no alpha channel: a picture with
//! transparency has to be blended onto a known colour before it is
//! encoded, or every transparent pixel comes out as whatever RGB sits
//! under the alpha, which is black in most files. The Fluxer theme fixes
//! a background to blend onto; the terminal theme takes the terminal's
//! own, so the terminal has to be asked for it.

use std::io::{Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::time::{Duration, Instant};

/// Nothing a terminal answers here is longer than this; a terminal
/// sending anything else is not answering our question.
const MAX_ANSWER: usize = 256;

/// Ask the terminal for its default background colour, waiting up to
/// `timeout` for the answer. Stdin must be in raw mode and nothing else
/// may be reading it. None when nothing answers or the answer is not a
/// colour we understand.
pub fn query(timeout: Duration) -> Option<[u8; 3]> {
    let mut out = std::io::stdout();
    // A Device Status Report follows the question: every terminal answers
    // that one, so a terminal that does not know OSC 11 ends the read
    // straight away instead of costing the whole timeout.
    out.write_all(b"\x1b]11;?\x1b\\\x1b[5n").ok()?;
    out.flush().ok()?;
    parse(&read_answer(timeout))
}

/// Bytes off stdin until the Device Status Report comes back, the
/// timeout runs out, or the answer grows past anything an answer can be.
fn read_answer(timeout: Duration) -> Vec<u8> {
    let deadline = Instant::now() + timeout;
    let stdin = std::io::stdin();
    let fd = stdin.as_raw_fd();
    let mut buf = Vec::with_capacity(64);
    while buf.len() < MAX_ANSWER {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() || !readable(fd, left) {
            break;
        }
        let mut chunk = [0u8; 64];
        match stdin.lock().read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
        if has_status_report(&buf) {
            break;
        }
    }
    buf
}

/// Whether `fd` has something to read within `timeout`, without blocking
/// past it: a terminal that ignores the question must not cost a
/// keystroke.
fn readable(fd: RawFd, timeout: Duration) -> bool {
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let ms = timeout.as_millis().min(i32::MAX as u128) as libc::c_int;
    unsafe { libc::poll(&mut pfd, 1, ms) > 0 }
}

/// The answer to the Device Status Report, `CSI <digits> n`: the end of
/// everything the terminal had to say about our question.
fn has_status_report(buf: &[u8]) -> bool {
    buf.windows(2)
        .enumerate()
        .filter(|(_, w)| w == b"\x1b[")
        .any(|(i, _)| {
            let rest = &buf[i + 2..];
            let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
            digits > 0 && rest.get(digits) == Some(&b'n')
        })
}

/// The colour out of an OSC 11 answer: `ESC ] 11 ; <colour> ST`, where
/// the colour is XParseColor's `rgb:r/g/b` (one to four hex digits each,
/// `rgba:` with an alpha we drop) or a legacy `#rgb`.
pub fn parse(buf: &[u8]) -> Option<[u8; 3]> {
    let start = buf.windows(5).position(|w| w == b"\x1b]11;")?;
    let body = &buf[start + 5..];
    let end = body
        .iter()
        .position(|&b| b == 0x07 || b == 0x1b)
        .unwrap_or(body.len());
    let spec = std::str::from_utf8(&body[..end]).ok()?.trim();
    parse_spec(spec)
}

/// An XParseColor colour.
fn parse_spec(spec: &str) -> Option<[u8; 3]> {
    if let Some(rest) = spec
        .strip_prefix("rgba:")
        .or_else(|| spec.strip_prefix("rgb:"))
    {
        let mut parts = rest.split('/');
        let rgb = [
            scale(parts.next()?)?,
            scale(parts.next()?)?,
            scale(parts.next()?)?,
        ];
        return Some(rgb);
    }
    // Legacy `#rgb`, `#rrggbb`, `#rrrgggbbb`, `#rrrrggggbbbb`; urxvt's
    // transparency extension puts an opacity in front, `[75]#ff00ff`.
    let hex = spec.rsplit(']').next()?.strip_prefix('#')?;
    if hex.len() % 3 != 0 || hex.is_empty() || hex.len() > 12 {
        return None;
    }
    let w = hex.len() / 3;
    Some([
        scale(&hex[..w])?,
        scale(&hex[w..2 * w])?,
        scale(&hex[2 * w..])?,
    ])
}

/// One to four hex digits as eight bits: `2b2b` and `2b` are both 0x2b.
fn scale(digits: &str) -> Option<u8> {
    if digits.is_empty() || digits.len() > 4 {
        return None;
    }
    let value = u32::from_str_radix(digits, 16).ok()?;
    let max = (1u32 << (4 * digits.len() as u32)) - 1;
    Some((value * 255 / max) as u8)
}

/// What `[ui] image_background` asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blend {
    /// The theme's background where the theme fixes one, the terminal's
    /// own otherwise: the default.
    Ask,
    /// Encode the picture as it is, alpha and all, and let the protocol
    /// lose it.
    None,
    /// This colour, whatever the theme and the terminal say.
    Fixed([u8; 3]),
}

/// Read the setting; None when it is not a colour, "auto" or "none".
pub fn setting(value: &str) -> Option<Blend> {
    match value.trim() {
        "" | "auto" => Some(Blend::Ask),
        "none" | "off" => Some(Blend::None),
        spec => parse_spec(spec)
            .or_else(|| parse_spec(&format!("#{spec}")))
            .map(Blend::Fixed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foots_answer_is_its_background() {
        // exactly what foot 1.27 sends back, terminator and all
        let raw = b"\x1b]11;rgb:0000/2b2b/3636\x1b\\\x1b[0n";
        assert_eq!(parse(raw), Some([0x00, 0x2b, 0x36]));
    }

    #[test]
    fn every_width_of_component_is_understood() {
        assert_eq!(parse(b"\x1b]11;rgb:00/2b/36\x07"), Some([0, 0x2b, 0x36]));
        assert_eq!(parse(b"\x1b]11;rgb:0/f/8\x07"), Some([0, 255, 136]));
        assert_eq!(
            parse(b"\x1b]11;rgb:ffff/ffff/ffff\x1b\\"),
            Some([255, 255, 255])
        );
        assert_eq!(
            parse(b"\x1b]11;rgba:00/2b/36/ff\x07"),
            Some([0, 0x2b, 0x36])
        );
    }

    #[test]
    fn legacy_and_urxvt_forms_are_understood() {
        assert_eq!(parse(b"\x1b]11;#002b36\x07"), Some([0, 0x2b, 0x36]));
        assert_eq!(parse(b"\x1b]11;#08f\x07"), Some([0, 136, 255]));
        assert_eq!(parse(b"\x1b]11;[75]#ff00ff\x07"), Some([255, 0, 255]));
    }

    #[test]
    fn nothing_is_read_out_of_a_non_answer() {
        assert_eq!(parse(b""), None);
        assert_eq!(parse(b"\x1b[0n"), None, "the status report alone");
        assert_eq!(parse(b"\x1b]10;rgb:0/0/0\x07"), None, "the foreground");
        assert_eq!(parse(b"\x1b]11;chartreuse\x07"), None, "a colour by name");
        assert_eq!(parse(b"\x1b]11;rgb:0000/2b2b\x07"), None, "two components");
    }

    #[test]
    fn the_status_report_ends_the_read() {
        assert!(!has_status_report(b"\x1b]11;rgb:0000/2b2b/3636\x1b\\"));
        assert!(has_status_report(b"\x1b]11;rgb:0/0/0\x1b\\\x1b[0n"));
        assert!(
            has_status_report(b"\x1b[3n"),
            "a terminal reporting trouble"
        );
        assert!(!has_status_report(b"\x1b[n"), "no digits is not a report");
    }
}

#[cfg(test)]
mod setting_tests {
    use super::*;

    #[test]
    fn the_setting_reads_a_colour_a_word_or_nothing() {
        assert_eq!(setting(""), Some(Blend::Ask));
        assert_eq!(setting("auto"), Some(Blend::Ask));
        assert_eq!(setting("none"), Some(Blend::None));
        assert_eq!(setting("off"), Some(Blend::None));
        assert_eq!(setting(" #002b36 "), Some(Blend::Fixed([0, 0x2b, 0x36])));
        assert_eq!(setting("002b36"), Some(Blend::Fixed([0, 0x2b, 0x36])));
        assert_eq!(setting("rgb:00/2b/36"), Some(Blend::Fixed([0, 0x2b, 0x36])));
        assert_eq!(setting("solarized"), None);
        assert_eq!(setting("#00zz36"), None);
    }
}
