//! Audio attachments are played by a program that does that well: the
//! bytes go to its stdin through a pipe, and it gets no terminal (stdout
//! and stderr to /dev/null), so the same thing works in a terminal
//! emulator and on a bare TTY where the client owns the console. The
//! command is the user's (`[media] audio_player`), or the first of the
//! usual players found on PATH.

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::api::types::MessageAttachmentResponse;

/// Players tried in turn when no command is configured, each told to read
/// stdin, play audio only, and leave the terminal alone.
const PLAYERS: &[&[&str]] = &[
    &["mpv", "--no-video", "--no-terminal", "--really-quiet", "-"],
    &["ffplay", "-nodisp", "-autoexit", "-loglevel", "error", "-"],
    &["pw-play", "-"],
    &["paplay"],
    &["aplay", "-q"],
];

pub fn attachment_is_audio(a: &MessageAttachmentResponse) -> bool {
    let mime = a.content_type.as_deref().unwrap_or("");
    if mime.starts_with("audio/") {
        return true;
    }
    let n = a.filename.to_lowercase();
    [
        ".mp3", ".ogg", ".oga", ".opus", ".wav", ".flac", ".m4a", ".aac", ".wma", ".weba", ".mka",
    ]
    .iter()
    .any(|ext| n.ends_with(ext))
}

/// A duration in seconds as `m:ss` (or `h:mm:ss`).
pub fn format_duration(secs: i64) -> String {
    let secs = secs.max(0);
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// The command to play audio with: the configured one (split on
/// whitespace), else the first player on PATH. None when there is none.
pub fn player_command(configured: &str) -> Option<Vec<String>> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    player_command_in(configured, &path)
}

fn player_command_in(configured: &str, path: &OsStr) -> Option<Vec<String>> {
    let configured: Vec<String> = configured.split_whitespace().map(str::to_string).collect();
    if !configured.is_empty() {
        return Some(configured);
    }
    PLAYERS
        .iter()
        .find(|argv| find_on_path(argv[0], path).is_some())
        .map(|argv| argv.iter().map(|s| s.to_string()).collect())
}

fn find_on_path(name: &str, path: &OsStr) -> Option<PathBuf> {
    if name.contains('/') {
        let p = Path::new(name);
        return is_executable(p).then(|| p.to_path_buf());
    }
    std::env::split_paths(path)
        .map(|dir| dir.join(name))
        .find(|p| is_executable(p))
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}

/// A player process with an attachment on its stdin.
#[derive(Debug)]
pub struct Player {
    child: tokio::process::Child,
    /// The program's name, for the status line.
    pub program: String,
    /// What plays: the attachment's file name.
    pub label: String,
    /// The attachment's cache key, so a second Ctrl+O on it stops instead
    /// of restarting.
    pub key: String,
}

impl Player {
    /// Start `argv` and feed it `bytes` on stdin from a background task.
    /// The process is killed if the client exits while it plays.
    pub fn start(argv: &[String], bytes: Vec<u8>, label: String, key: String) -> io::Result<Self> {
        let (program, args) = argv
            .split_first()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty player command"))?;
        let mut child = tokio::process::Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| io::Error::new(e.kind(), format!("{program}: {e}")))?;
        if let Some(mut stdin) = child.stdin.take() {
            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;
                // The player may stop reading early (stopped, or done with
                // the part it wanted); that is not an error worth keeping.
                let _ = stdin.write_all(&bytes).await;
                let _ = stdin.shutdown().await;
            });
        }
        Ok(Self {
            child,
            program: Path::new(program)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(program)
                .to_string(),
            label,
            key,
        })
    }

    /// Ask the player to stop: SIGTERM, which every player handles.
    pub fn stop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.child.id() {
            // SAFETY: a plain signal to a process we spawned and still own.
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGTERM);
            }
            return;
        }
        let _ = self.child.start_kill();
    }

    /// Whether the process has ended (reaping it).
    pub fn finished(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)) | Err(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_command_wins_and_is_split_on_whitespace() {
        let argv = player_command_in("  sox -q -t mp3 -   ", OsStr::new("")).unwrap();
        assert_eq!(argv, ["sox", "-q", "-t", "mp3", "-"]);
    }

    #[cfg(unix)]
    #[test]
    fn detects_the_first_player_on_path() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        // only "paplay" exists, and something non-executable called mpv
        std::fs::write(dir.path().join("mpv"), "").unwrap();
        let p = dir.path().join("paplay");
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        let argv = player_command_in("", dir.path().as_os_str()).unwrap();
        assert_eq!(argv, ["paplay"]);
        assert!(player_command_in("", OsStr::new("/nonexistent")).is_none());
    }

    #[test]
    fn audio_attachments_by_mime_or_name() {
        let mut a = MessageAttachmentResponse {
            filename: "voice-message.ogg".into(),
            ..Default::default()
        };
        assert!(attachment_is_audio(&a));
        a.filename = "clip.mp4".into();
        assert!(!attachment_is_audio(&a));
        a.content_type = Some("audio/mpeg".into());
        assert!(attachment_is_audio(&a));
    }

    #[test]
    fn durations_read_like_a_clock() {
        assert_eq!(format_duration(7), "0:07");
        assert_eq!(format_duration(754), "12:34");
        assert_eq!(format_duration(3600 + 61), "1:01:01");
        assert_eq!(format_duration(-3), "0:00");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_player_gets_the_bytes_on_stdin_and_is_reaped() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        let argv: Vec<String> = ["sh", "-c", &format!("cat > {}", out.display())]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut player =
            Player::start(&argv, b"RIFF....".to_vec(), "x.wav".into(), "k".into()).unwrap();
        assert_eq!(player.program, "sh");
        for _ in 0..200 {
            if player.finished() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(player.finished());
        assert_eq!(std::fs::read(&out).unwrap(), b"RIFF....");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dropping_the_player_kills_it() {
        let argv: Vec<String> = ["sh", "-c", "cat > /dev/null; sleep 30"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let player = Player::start(&argv, Vec::new(), "x".into(), "k".into()).unwrap();
        let pid = player.child.id().unwrap();
        drop(player);
        for _ in 0..200 {
            // SAFETY: signal 0 only checks whether the pid exists.
            if unsafe { libc::kill(pid as libc::pid_t, 0) } != 0 || is_zombie(pid) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("player survived the drop");
    }

    #[cfg(unix)]
    fn is_zombie(pid: u32) -> bool {
        std::fs::read_to_string(format!("/proc/{pid}/status"))
            .map(|s| {
                s.lines()
                    .any(|l| l.starts_with("State:") && l.contains('Z'))
            })
            .unwrap_or(true)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stop_ends_a_player_that_keeps_running() {
        let argv: Vec<String> = ["sh", "-c", "cat > /dev/null; sleep 30"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut player = Player::start(&argv, Vec::new(), "x".into(), "k".into()).unwrap();
        assert!(!player.finished());
        player.stop();
        for _ in 0..200 {
            if player.finished() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("player did not stop");
    }
}
