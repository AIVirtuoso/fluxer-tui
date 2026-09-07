//! The debug log: what the client did and what the server answered, as
//! lines a bug report can carry. It records the shape of things, never
//! their content: event kinds, ids, sizes, timings, the client's own
//! status-line messages and errors. No message text, no names, no
//! addresses, no file names, and never the token.
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

/// Start writing the log to `path`, appending to what is there. Lines
/// logged before this point are written out first.
pub fn init(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(
        file,
        "--- fluxer-tui {} started {}",
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
        "fluxer-tui {} debug snapshot {}",
        env!("CARGO_PKG_VERSION"),
        chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    )?;
    for (name, value) in facts {
        writeln!(file, "{name}: {value}")?;
    }
    writeln!(file)?;
    for line in recent(RING_LINES) {
        writeln!(file, "{line}")?;
    }
    Ok(path)
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

/// A path with the home directory as `~`: the user's name has no place in
/// the log.
pub fn scrub_path(text: &str) -> String {
    match dirs::home_dir() {
        Some(home) => text.replace(&*home.to_string_lossy(), "~"),
        None => text.to_string(),
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
    fn hosts_and_home_directories_are_all_that_is_kept_of_addresses_and_paths() {
        assert_eq!(
            url_host("https://user:pw@cdn.example.org/a/b.png?x=1"),
            "https://cdn.example.org"
        );
        assert_eq!(
            url_host("wss://gateway.fluxer.app/?v=1"),
            "wss://gateway.fluxer.app"
        );
        assert_eq!(url_host("no scheme here"), "no scheme here");
        if let Some(home) = dirs::home_dir() {
            let inside = format!("{}/pictures/cat.png", home.display());
            assert_eq!(scrub_path(&inside), "~/pictures/cat.png");
        }
        assert_eq!(scrub_path("/etc/hosts"), "/etc/hosts");
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
        assert!(written.starts_with("--- fluxer-tui"));
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
}
