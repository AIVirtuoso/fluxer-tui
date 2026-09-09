//! Notifications for messages that concern the user, handed to a program
//! that delivers them: `notify-send` (libnotify) on a desktop, or GNU
//! `mail` on the console, where "You have new mail" is the notification.
//! The client only decides what to say; `[ui] notifications` says which.
//! A sound goes with them, through the same kind of player that plays
//! audio attachments (a program reading the sound from stdin), so it is
//! heard in a terminal emulator and on the console alike.

use crate::config::NotifyMode;
use std::process::{Command, Stdio};

/// What to tell the user about one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// "author in #channel" or "author (direct message)".
    pub title: String,
    /// The message text, cut short, with a note about its files.
    pub body: String,
    /// "#channel · Community" or "Direct message", for the mail's footer.
    pub place: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// `notify-send <title> <body>`.
    Desktop,
    /// `mail -s <subject> <recipient>` with the body on stdin.
    Mail,
}

/// Whether a desktop is there to show a notification on.
pub fn has_display() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some()
}

/// Which program delivers notifications for `mode` here. Auto picks
/// notify-send where there is a display and nothing elsewhere; mail is
/// only ever used when asked for by name, since it puts messages in the
/// user's mailbox. Whether the program is there is found out when it is
/// run, so a missing one is reported rather than passed over in silence.
pub fn backend(mode: NotifyMode, display: bool) -> Option<Backend> {
    match mode {
        NotifyMode::Off => None,
        NotifyMode::Desktop => Some(Backend::Desktop),
        NotifyMode::Mail => Some(Backend::Mail),
        NotifyMode::Auto => display.then_some(Backend::Desktop),
    }
}

/// The recipient of mailed notifications: the configured one, else the
/// login user.
pub fn mail_recipient(configured: &str) -> String {
    let to = configured.trim();
    if !to.is_empty() {
        return to.to_string();
    }
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "root".to_string())
}

fn desktop_args(n: &Notification) -> Vec<String> {
    vec![
        "--app-name=fluxter".to_string(),
        n.title.clone(),
        n.body.clone(),
    ]
}

fn mail_args(n: &Notification, to: &str) -> Vec<String> {
    vec![
        "-s".to_string(),
        format!("[fluxter] {}", n.title),
        to.to_string(),
    ]
}

fn mail_body(n: &Notification) -> String {
    format!(
        "{}\n\n{} \u{00B7} {}\n",
        n.body,
        n.place,
        chrono::Local::now().format("%Y-%m-%d %H:%M")
    )
}

/// Deliver `n` through `backend`, in the background; the status line gets
/// a word only when the program cannot be started.
pub fn send(
    backend: Backend,
    n: Notification,
    mail_to: String,
    mail_command: String,
    desktop_command: String,
    event_tx: tokio::sync::mpsc::UnboundedSender<crate::events::AppEvent>,
) {
    let or_default = |configured: &str, default: &str| {
        let c = configured.trim();
        if c.is_empty() { default } else { c }.to_string()
    };
    tokio::task::spawn_blocking(move || {
        let (program, args, stdin) = match backend {
            Backend::Desktop => (
                or_default(&desktop_command, "notify-send"),
                desktop_args(&n),
                None,
            ),
            Backend::Mail => (
                or_default(&mail_command, "mail"),
                mail_args(&n, &mail_to),
                Some(mail_body(&n)),
            ),
        };
        let mut child = match Command::new(&program)
            .args(&args)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(err) => {
                let hint = match backend {
                    Backend::Desktop => " (libnotify, plus a notification daemon)",
                    Backend::Mail => " (GNU mailutils)",
                };
                let _ = event_tx.send(crate::events::AppEvent::SetStatus(format!(
                    "Notifications: cannot run {program}{hint}: {err}"
                )));
                return;
            }
        };
        if let (Some(body), Some(mut pipe)) = (stdin, child.stdin.take()) {
            use std::io::Write;
            let _ = pipe.write_all(body.as_bytes());
        }
        let status = child.wait();
        crate::debug::log(
            "notify",
            format!(
                "{program} {}",
                match status {
                    Ok(s) if s.success() => "ran".to_string(),
                    Ok(s) => format!("failed: {s}"),
                    Err(e) => format!("could not be waited for: {e}"),
                }
            ),
        );
    });
}

/// The built-in notification sound: a short two-note chime as a WAV
/// file, made here so that no sound file has to be shipped or found.
/// 22.05 kHz, 16-bit, mono, about a quarter of a second.
pub fn chime_wav() -> Vec<u8> {
    const RATE: u32 = 22_050;
    const NOTE: usize = 2_800; // samples per note, ~127 ms
    let notes: [(f32, f32); 2] = [(880.0, 0.35), (1318.5, 0.3)];
    let mut samples: Vec<i16> = Vec::with_capacity(NOTE * notes.len());
    for (hz, gain) in notes {
        for i in 0..NOTE {
            let t = i as f32 / RATE as f32;
            // a soft attack, an exponential decay and a short release,
            // so it rings rather than clicks
            let attack = (i as f32 / 200.0).min(1.0);
            let release = ((NOTE - i) as f32 / 200.0).min(1.0);
            let decay = (-t * 14.0).exp();
            let v = (t * hz * std::f32::consts::TAU).sin() * gain * attack * release * decay;
            samples.push((v * i16::MAX as f32) as i16);
        }
    }
    let data_len = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes()); // bytes per second
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        wav.extend_from_slice(&s.to_le_bytes());
    }
    wav
}

/// The bytes of the notification sound: the configured file, or the
/// built-in chime when none is named.
pub fn sound_bytes(file: &str) -> std::io::Result<Vec<u8>> {
    let file = file.trim();
    if file.is_empty() {
        return Ok(chime_wav());
    }
    let path = if let Some(rest) = file.strip_prefix("~/") {
        dirs::home_dir()
            .map(|h| h.join(rest))
            .unwrap_or_else(|| std::path::PathBuf::from(file))
    } else {
        std::path::PathBuf::from(file)
    };
    std::fs::read(&path)
        .map_err(|e| std::io::Error::new(e.kind(), format!("{}: {e}", path.display())))
}

/// Play `bytes` through `argv`, a player reading its stdin, in the
/// background; the status line gets a word only when the program cannot
/// be started. The player is left to finish on its own.
pub fn play_sound(
    argv: Vec<String>,
    bytes: Vec<u8>,
    event_tx: tokio::sync::mpsc::UnboundedSender<crate::events::AppEvent>,
) {
    tokio::task::spawn_blocking(move || {
        let Some((program, args)) = argv.split_first() else {
            return;
        };
        let mut child = match Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(err) => {
                let _ = event_tx.send(crate::events::AppEvent::SetStatus(format!(
                    "Notification sound: cannot run {program}: {err}"
                )));
                return;
            }
        };
        if let Some(mut pipe) = child.stdin.take() {
            use std::io::Write;
            // the player may stop reading early; that is its business
            let _ = pipe.write_all(&bytes);
        }
        let status = child.wait();
        crate::debug::log(
            "notify",
            format!(
                "sound: {program} {}",
                match status {
                    Ok(s) if s.success() => "ran".to_string(),
                    Ok(s) => format!("failed: {s}"),
                    Err(e) => format!("could not be waited for: {e}"),
                }
            ),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n() -> Notification {
        Notification {
            title: "ann in #general".into(),
            body: "hello there".into(),
            place: "#general \u{00B7} Linux Hub".into(),
        }
    }

    #[test]
    fn auto_uses_the_desktop_only_and_mail_is_opt_in() {
        assert_eq!(backend(NotifyMode::Auto, true), Some(Backend::Desktop));
        assert_eq!(
            backend(NotifyMode::Auto, false),
            None,
            "no mail unless asked"
        );
        assert_eq!(backend(NotifyMode::Desktop, false), Some(Backend::Desktop));
        assert_eq!(backend(NotifyMode::Mail, true), Some(Backend::Mail));
        assert_eq!(backend(NotifyMode::Off, true), None);
    }

    /// A script standing in for the program records how it was called.
    #[cfg(unix)]
    fn stand_in(dir: &std::path::Path, name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let log = dir.join(format!("{name}.log"));
        let script = dir.join(name);
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > {0}\ncat >> {0}\n",
                log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        (script, log)
    }

    #[cfg(unix)]
    async fn wait_for(log: &std::path::Path) -> String {
        for _ in 0..300 {
            if let Ok(s) = std::fs::read_to_string(log)
                && !s.is_empty()
            {
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                return std::fs::read_to_string(log).unwrap();
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("{} was never written", log.display());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn mail_gets_the_subject_the_recipient_and_the_body_on_stdin() {
        let dir = tempfile::tempdir().unwrap();
        let (script, log) = stand_in(dir.path(), "mail");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        send(
            Backend::Mail,
            n(),
            "zero".into(),
            script.display().to_string(),
            String::new(),
            tx,
        );
        let got = wait_for(&log).await;
        assert!(
            got.starts_with("-s\n[fluxter] ann in #general\nzero\nhello there\n\n#general"),
            "{got:?}"
        );
        assert!(rx.try_recv().is_err(), "no complaint when the program ran");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_desktop_gets_title_and_body_and_a_missing_program_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let (script, log) = stand_in(dir.path(), "notify-send");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        send(
            Backend::Desktop,
            n(),
            String::new(),
            String::new(),
            script.display().to_string(),
            tx.clone(),
        );
        let got = wait_for(&log).await;
        assert_eq!(got, "--app-name=fluxter\nann in #general\nhello there\n");
        send(
            Backend::Desktop,
            n(),
            String::new(),
            String::new(),
            dir.path().join("no-such-program").display().to_string(),
            tx,
        );
        let status = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("a status")
            .expect("a status");
        match status {
            crate::events::AppEvent::SetStatus(s) => {
                assert!(
                    s.starts_with("Notifications: cannot run ") && s.contains("libnotify"),
                    "{s}"
                )
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_chime_is_a_well_formed_short_wav() {
        let wav = chime_wav();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[36..40], b"data");
        let riff = u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize;
        assert_eq!(riff + 8, wav.len());
        let data = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
        assert_eq!(data + 44, wav.len());
        let rate = u32::from_le_bytes(wav[24..28].try_into().unwrap());
        let seconds = data as f32 / 2.0 / rate as f32;
        assert!((0.2..0.5).contains(&seconds), "{seconds}s");
        // it starts and ends in silence: no click
        assert_eq!(&wav[44..46], &[0, 0]);
        let last = i16::from_le_bytes(wav[wav.len() - 2..].try_into().unwrap());
        assert!(last.abs() < 200, "{last}");
        assert_eq!(sound_bytes("  ").unwrap(), wav);
        assert!(sound_bytes("/no/such/sound.wav").is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_sound_goes_to_the_player_on_stdin_and_a_missing_player_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let (script, log) = stand_in(dir.path(), "player");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        play_sound(
            vec![script.display().to_string(), "-q".into()],
            b"RIFFsound".to_vec(),
            tx.clone(),
        );
        let got = wait_for(&log).await;
        assert_eq!(got, "-q\nRIFFsound");
        assert!(rx.try_recv().is_err(), "no complaint when the program ran");
        play_sound(
            vec![dir.path().join("no-such-player").display().to_string()],
            Vec::new(),
            tx,
        );
        let status = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("a status")
            .expect("a status");
        match status {
            crate::events::AppEvent::SetStatus(s) => {
                assert!(s.starts_with("Notification sound: cannot run "), "{s}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_programs_get_the_notification_as_they_expect_it() {
        let d = desktop_args(&n());
        assert_eq!(d, ["--app-name=fluxter", "ann in #general", "hello there"]);
        let m = mail_args(&n(), "zero");
        assert_eq!(m, ["-s", "[fluxter] ann in #general", "zero"]);
        let body = mail_body(&n());
        assert!(body.starts_with("hello there\n\n#general \u{00B7} Linux Hub \u{00B7} "));
        assert_eq!(mail_recipient("  bob "), "bob");
        assert!(!mail_recipient("").is_empty());
    }
}
