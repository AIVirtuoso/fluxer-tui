//! The debug log: what the client did and what the server answered, as
//! lines a bug report can carry. It records the shape of things, never
//! their content: event kinds, ids, sizes, timings, the client's own
//! status-line messages and errors. No message text, no names, no
//! addresses, no file names or paths, and never the token.
//!
//! The last few hundred lines are always kept in memory for the debug
//! panel (`/debug`); `--debug` also writes every line to a file.

use serde_json::Value;
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

/// Lines kept in memory for the panel and for snapshots.
pub const RING_LINES: usize = 300;
/// A line longer than this is cut: a shape line of a big payload is still
/// readable at this length, and nothing useful hides beyond it.
const MAX_LINE: usize = 2000;

struct Sink {
    ring: VecDeque<String>,
    file: Option<File>,
    path: Option<PathBuf>,
}

fn sink() -> MutexGuard<'static, Sink> {
    static SINK: OnceLock<Mutex<Sink>> = OnceLock::new();
    SINK.get_or_init(|| {
        Mutex::new(Sink {
            ring: VecDeque::with_capacity(RING_LINES),
            file: None,
            path: None,
        })
    })
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The default log file: `$XDG_STATE_HOME/fluxer-tui/debug.log`, which is
/// `~/.local/state/fluxer-tui/debug.log` unless the environment says
/// otherwise; the cache directory when there is no state directory.
pub fn default_path() -> PathBuf {
    dirs::state_dir()
        .or_else(dirs::cache_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("fluxer-tui")
        .join("debug.log")
}

/// Make sure the folder the log and the snapshots go to is there, even
/// when no log is kept this time: someone looking for the log after the
/// fact should find the folder, and a `/debug save` in a session that
/// was started without `--debug` writes into it. The path comes back.
pub fn ensure_log_dir() -> std::io::Result<PathBuf> {
    ensure_dir_of(&default_path())
}

fn ensure_dir_of(path: &Path) -> std::io::Result<PathBuf> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("the log path has no folder"))?;
    std::fs::create_dir_all(parent)?;
    Ok(parent.to_path_buf())
}

/// Start writing the log to `path`, appending to what is there. Lines
/// logged before this point are written out first.
pub fn init(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(
        file,
        "--- fluxter {} started {}",
        env!("CARGO_PKG_VERSION"),
        chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    )?;
    let mut s = sink();
    for line in &s.ring {
        writeln!(file, "{line}")?;
    }
    s.file = Some(file);
    s.path = Some(path.to_path_buf());
    Ok(())
}

/// The file the log goes to, when it goes to one.
pub fn path() -> Option<PathBuf> {
    sink().path.clone()
}

pub fn enabled() -> bool {
    sink().path.is_some()
}

/// Where the log is, for a person to read: the file being written to, or
/// the one `--debug` would use, with the home directory as `~`. For the
/// screen and `--help` only; the log itself never carries a path, and
/// `save_snapshot` scrubs this back to `<path>` before it reaches a file
/// that someone might attach to a bug report.
pub fn log_location() -> String {
    abbreviate_home(&path().unwrap_or_else(default_path), dirs::home_dir())
}

fn abbreviate_home(path: &Path, home: Option<PathBuf>) -> String {
    let text = path.display().to_string();
    home.map(|h| h.display().to_string())
        .filter(|h| !h.is_empty())
        .and_then(|h| text.strip_prefix(&h).map(|rest| format!("~{rest}")))
        .unwrap_or(text)
}

/// One line: the time, the area it concerns (gateway, http, media, ...),
/// and what happened.
pub fn log(area: &str, message: impl AsRef<str>) {
    let message = message.as_ref();
    let mut line = format!(
        "{} {:<8} {}",
        chrono::Local::now().format("%H:%M:%S%.3f"),
        area,
        message
    );
    if line.len() > MAX_LINE {
        let mut cut = MAX_LINE;
        while !line.is_char_boundary(cut) {
            cut -= 1;
        }
        line.truncate(cut);
        line.push('\u{2026}');
    }
    let mut s = sink();
    if s.ring.len() >= RING_LINES {
        s.ring.pop_front();
    }
    s.ring.push_back(line.clone());
    if let Some(file) = s.file.as_mut() {
        let _ = writeln!(file, "{line}");
    }
}

/// The last `n` lines, oldest first.
pub fn recent(n: usize) -> Vec<String> {
    let s = sink();
    let skip = s.ring.len().saturating_sub(n);
    s.ring.iter().skip(skip).cloned().collect()
}

/// The facts and the recent lines, written to a file next to the log
/// (next to where the log would be, when none is kept) for a bug
/// report; the path comes back.
pub fn save_snapshot(facts: &[(String, String)]) -> std::io::Result<PathBuf> {
    let path = path().unwrap_or_else(default_path).with_file_name(format!(
        "debug-{}.txt",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = File::create(&path)?;
    writeln!(
        file,
        "fluxter {} debug snapshot {}",
        env!("CARGO_PKG_VERSION"),
        chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    )?;
    for (name, value) in facts {
        writeln!(file, "{name}: {}", scrub_private(value))?;
    }
    writeln!(file)?;
    for line in recent(RING_LINES) {
        writeln!(file, "{line}")?;
    }
    Ok(path)
}

/// A map of a drawn frame, one character per cell, for the log: where the
/// borders, pictures and text are, not what the text says. `#` is a
/// box-drawing cell, `P` a picture (media marker) cell, `e` a custom
/// emoji cell, `T` a text cell and a space a blank cell (the second cell
/// of a wide character among them). A single blank between two text cells counts as
/// text, so a row shows where text is and where it ends but not the
/// lengths of its words.
pub fn frame_map(buf: &ratatui::buffer::Buffer) -> Vec<String> {
    let area = buf.area;
    let mut rows = Vec::with_capacity(area.height as usize);
    for y in area.top()..area.bottom() {
        let mut row: Vec<char> = Vec::with_capacity(area.width as usize);
        for x in area.left()..area.right() {
            let cell = &buf[(x, y)];
            let symbol = cell.symbol();
            let class = if symbol == "\u{2800}" {
                if crate::app::media_marker(cell.style()).is_some() {
                    'P'
                } else if crate::app::custom_emoji_marker_slot(cell.style()).is_some() {
                    'e'
                } else {
                    ' '
                }
            } else if symbol.chars().all(char::is_whitespace) {
                ' '
            } else if symbol
                .chars()
                .next()
                .is_some_and(|c| ('\u{2500}'..='\u{257F}').contains(&c))
            {
                '#'
            } else {
                'T'
            };
            row.push(class);
        }
        for i in 1..row.len().saturating_sub(1) {
            if row[i] == ' ' && row[i - 1] == 'T' && row[i + 1] == 'T' {
                row[i] = 'T';
            }
        }
        rows.push(row.into_iter().collect());
    }
    rows
}

/// Panics go to the log, with a backtrace, before the previous hook
/// (which restores the terminal) runs.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log("panic", info.to_string());
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        for line in backtrace.lines().take(60) {
            log("panic", line.trim_end());
        }
        previous(info);
    }));
}

/// A status line without what is private in it: a word that is a path
/// (`~/pics/cat.png`, `/dev/dri/card0`) becomes `<path>`, one that names
/// a file (`cat.png`) becomes `<file>`, and an address keeps only its
/// host. The client's status lines mention the files it attached or
/// could not read, and where they are helps nobody but their owner. A
/// version-like word (`0.7.5`) or a plain word stays.
pub fn scrub_private(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        let word_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let (word, tail) = rest.split_at(word_end);
        let space_end = tail
            .find(|c: char| !c.is_whitespace())
            .unwrap_or(tail.len());
        let (spaces, after) = tail.split_at(space_end);
        out.push_str(&scrub_word(word));
        out.push_str(spaces);
        rest = after;
    }
    out
}

fn scrub_word(word: &str) -> String {
    const PUNCT: &[char] = &[
        '(', ')', '[', ']', '"', '\'', ',', ':', ';', '.', '\u{2026}', '!', '?',
    ];
    let core = word.trim_matches(PUNCT);
    let lead = &word[..word.len() - word.trim_start_matches(PUNCT).len()];
    let trail = &word[lead.len() + core.len()..];
    if core.contains("://") {
        return format!("{lead}{}{trail}", url_host(core));
    }
    if core.contains('/') || core.starts_with('~') {
        return format!("{lead}<path>{trail}");
    }
    let looks_like_file = core.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty()
            && (1..=5).contains(&ext.len())
            && ext.bytes().all(|b| b.is_ascii_alphanumeric())
            && ext.bytes().any(|b| b.is_ascii_alphabetic())
    });
    if looks_like_file {
        format!("{lead}<file>{trail}")
    } else {
        word.to_string()
    }
}

/// Scheme and host of a URL, nothing after: which server was talked to,
/// not what was asked of it.
pub fn url_host(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or(host);
    if scheme.is_empty() {
        host.to_string()
    } else {
        format!("{scheme}://{host}")
    }
}

/// The shape of a JSON value, three levels of keys deep: field names and
/// types, the length of strings, the size of arrays (and the shape of
/// their first element near the top), and the value of `id` fields when
/// they are snowflakes. No other content. This is what shows that a
/// field came as `null` or as a list when a map was expected, without
/// copying what the field held.
pub fn shape(value: &Value) -> String {
    shape_depth(value, None, 0)
}

fn shape_depth(value: &Value, key: Option<&str>, depth: usize) -> String {
    let is_id = key.is_some_and(|k| k == "id" || k.ends_with("_id"));
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(_) => "bool".to_string(),
        Value::Number(n) if is_id => n.to_string(),
        Value::Number(_) => "num".to_string(),
        Value::String(s) if is_id && !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => {
            s.clone()
        }
        Value::String(s) => format!("str({})", s.chars().count()),
        Value::Array(items) => match items.first() {
            Some(first) if depth < 2 => {
                format!(
                    "arr({}) of {}",
                    items.len(),
                    shape_depth(first, None, depth + 1)
                )
            }
            _ => format!("arr({})", items.len()),
        },
        Value::Object(map) => {
            if depth >= 3 {
                return format!("obj({})", map.len());
            }
            let fields: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{k}:{}", shape_depth(v, Some(k), depth + 1)))
                .collect();
            format!("{{{}}}", fields.join(" "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_log_location_shows_the_home_directory_as_a_tilde() {
        let home = PathBuf::from("/home/someone");
        assert_eq!(
            abbreviate_home(
                &home.join(".local/state/fluxer-tui/debug.log"),
                Some(home.clone())
            ),
            "~/.local/state/fluxer-tui/debug.log"
        );
        // a log put somewhere else, and a run with no home, stay as they are
        let elsewhere = Path::new("/var/log/fluxer.log");
        assert_eq!(
            abbreviate_home(elsewhere, Some(home)),
            "/var/log/fluxer.log"
        );
        assert_eq!(abbreviate_home(elsewhere, None), "/var/log/fluxer.log");
    }

    #[test]
    fn a_snapshot_fact_naming_a_path_is_scrubbed() {
        // facts go on screen with the real path and into the file without it
        assert_eq!(
            scrub_private("kept at ~/.local/state/fluxer-tui/debug.log, snapshots beside it"),
            "kept at <path>, snapshots beside it"
        );
    }

    #[test]
    fn a_shape_shows_fields_types_sizes_and_ids_but_no_content() {
        let s = shape(&json!({
            "id": "1546536121037488128",
            "channel_id": 42,
            "content": "the secret message",
            "author": {"id": "7", "username": "somebody", "email": "a@b.c"},
            "mentions": [{"id": "9", "username": "x"}],
            "channel_overrides": null,
            "pinned": false,
            "attachments": []
        }));
        assert_eq!(
            s,
            "{attachments:arr(0) author:{email:str(5) id:7 username:str(8)} \
             channel_id:42 channel_overrides:null content:str(18) \
             id:1546536121037488128 mentions:arr(1) of {id:9 username:str(1)} pinned:bool}"
        );
        assert!(!s.contains("secret"));
        assert!(!s.contains("somebody"));
        assert!(!s.contains("a@b.c"));
    }

    #[test]
    fn deeper_levels_are_only_counted_and_an_id_that_is_not_a_snowflake_is_hidden() {
        let s = shape(&json!({
            "guilds": [{"id": "1", "properties": {"name": "Linux Hub", "owner_id": "5"}, "roles": [1, 2]}],
            "session_id": "abc123def"
        }));
        assert_eq!(
            s,
            "{guilds:arr(1) of {id:1 properties:obj(2) roles:arr(2)} session_id:str(9)}"
        );
    }

    #[test]
    fn file_names_and_paths_in_status_lines_are_not_kept() {
        assert_eq!(
            scrub_private("Attached cat.png (136 B). Enter sends, Ctrl+X removes."),
            "Attached <file> (136 B). Enter sends, Ctrl+X removes."
        );
        assert_eq!(
            scrub_private("Reading ~/pics/holiday 2026.jpeg\u{2026}"),
            "Reading <path> <file>\u{2026}"
        );
        assert_eq!(scrub_private("Removed /tmp/notes.toml."), "Removed <path>.");
        assert_eq!(
            scrub_private("Console mode unavailable: /dev/dri/card0: permission denied"),
            "Console mode unavailable: <path>: permission denied"
        );
        assert_eq!(
            scrub_private("fluxter 0.7.5 needs mpv (see https://x.org/a.html)"),
            "fluxter 0.7.5 needs mpv (see https://x.org)"
        );
        assert_eq!(
            scrub_private("Failed to load: 504 Gateway Timeout."),
            "Failed to load: 504 Gateway Timeout."
        );
    }

    #[test]
    fn hosts_are_all_that_is_kept_of_addresses() {
        assert_eq!(
            url_host("https://user:pw@cdn.example.org/a/b.png?x=1"),
            "https://cdn.example.org"
        );
        assert_eq!(
            url_host("wss://gateway.fluxer.app/?v=1"),
            "wss://gateway.fluxer.app"
        );
        assert_eq!(url_host("no scheme here"), "no scheme here");
    }

    #[test]
    fn the_log_folder_is_made_on_its_own_and_the_default_path_sits_in_one() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir
            .path()
            .join("state")
            .join("fluxer-tui")
            .join("debug.log");
        let made = ensure_dir_of(&file).unwrap();
        assert_eq!(made, dir.path().join("state").join("fluxer-tui"));
        assert!(made.is_dir());
        assert!(!file.exists(), "the folder, not the file");
        assert!(ensure_dir_of(&file).is_ok(), "a second time is fine");
        let default = default_path();
        assert_eq!(default.file_name().unwrap(), "debug.log");
        assert_eq!(default.parent().unwrap().file_name().unwrap(), "fluxer-tui");
    }

    #[test]
    fn the_ring_keeps_the_last_lines_and_a_file_gets_every_line() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("nested").join("debug.log");
        log("test", "before the file");
        init(&file).unwrap();
        assert_eq!(path(), Some(file.clone()));
        for i in 0..(RING_LINES + 5) {
            log("test", format!("line {i}"));
        }
        let recent = recent(3);
        assert_eq!(recent.len(), 3);
        assert!(
            recent[2].ends_with(&format!("line {}", RING_LINES + 4)),
            "{:?}",
            recent
        );
        assert!(recent[0].contains("test     line"), "{:?}", recent);
        let written = std::fs::read_to_string(&file).unwrap();
        assert!(written.starts_with("--- fluxter"));
        assert!(written.contains("before the file"));
        assert!(written.contains(&format!("line {}", RING_LINES + 4)));

        let long = "x".repeat(MAX_LINE + 50);
        log("test", &long);
        let last = recent_line();
        assert!(last.chars().count() <= MAX_LINE + 1);
        assert!(last.ends_with('\u{2026}'));

        let snapshot = save_snapshot(&[("version".into(), "1".into())]).unwrap();
        let body = std::fs::read_to_string(&snapshot).unwrap();
        assert!(body.contains("version: 1"));
        let _ = std::fs::remove_file(snapshot);
    }

    fn recent_line() -> String {
        recent(1).pop().unwrap()
    }

    #[test]
    fn a_frame_map_shows_where_things_are_but_not_the_words() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 3));
        buf.set_string(
            0,
            0,
            "\u{250C}\u{2500}\u{2500}\u{2510}",
            ratatui::style::Style::default(),
        );
        buf.set_string(0, 1, "hi you  \u{4F60}", ratatui::style::Style::default());
        buf.set_string(
            0,
            2,
            "\u{2800}\u{2800}\u{2800}",
            crate::app::media_marker_style(3, 1),
        );
        buf.set_string(4, 2, "\u{2800}", crate::app::custom_emoji_marker_style(7));
        let map = frame_map(&buf);
        assert_eq!(map[0], "####        ");
        assert_eq!(map[1], "TTTTTT  T   ");
        assert_eq!(map[2], "PPP e       ");
    }
}
