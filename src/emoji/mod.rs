//! Emoji shortcodes the way Fluxer spells them.
//!
//! The `emojis` crate ships GitHub (gemoji) names; Fluxer uses Discord-style
//! names, and the two disagree for about 900 of Fluxer's 2300 names
//! (`:slight_smile:` vs `slightly_smiling_face`, `:thumbsup:` vs `+1`, ...).
//! [`resolve`] consults the generated Fluxer alias table first and the crate
//! second, so anything the web app turns into an emoji does the same here.

mod aliases;

use std::collections::HashMap;
use std::sync::LazyLock;

pub use aliases::FLUXER_EMOJI_ALIASES;

static ALIAS_MAP: LazyLock<HashMap<&'static str, &'static str>> =
    LazyLock::new(|| FLUXER_EMOJI_ALIASES.iter().copied().collect());

/// Every searchable (name, emoji) pair: Fluxer aliases plus the crate's own
/// shortcodes, one entry per name.
static ALL_NAMES: LazyLock<Vec<(&'static str, &'static str)>> = LazyLock::new(|| {
    let mut v: Vec<(&'static str, &'static str)> = FLUXER_EMOJI_ALIASES.to_vec();
    for e in emojis::iter() {
        for code in e.shortcodes() {
            if !ALIAS_MAP.contains_key(code) {
                v.push((code, e.as_str()));
            }
        }
    }
    v
});

/// The unicode emoji for a shortcode name (without the colons), or None.
pub fn resolve(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        return None;
    }
    if let Some(e) = ALIAS_MAP.get(name) {
        return Some(e);
    }
    if let Some(e) = emojis::get_by_shortcode(name) {
        return Some(e.as_str());
    }
    let lower = name.to_ascii_lowercase();
    if lower != name {
        return resolve(&lower);
    }
    None
}

/// `:name::skin-tone-N:` (N in 1..=5) as the web app writes it.
fn resolve_with_skin_tone(name: &str, tone: u8) -> Option<&'static str> {
    use emojis::SkinTone;
    let base = resolve(name)?;
    let tone = match tone {
        1 => SkinTone::Light,
        2 => SkinTone::MediumLight,
        3 => SkinTone::Medium,
        4 => SkinTone::MediumDark,
        5 => SkinTone::Dark,
        _ => return None,
    };
    emojis::get(base)
        .and_then(|e| e.with_skin_tone(tone))
        .map(|e| e.as_str())
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-')
}

/// One piece of a text run: literal text or a resolved shortcode.
#[derive(Debug, PartialEq, Eq)]
pub enum Segment<'a> {
    Text(&'a str),
    Emoji {
        /// The `:name:` (or `:name::skin-tone-N:`) as written.
        raw: &'a str,
        emoji: &'static str,
    },
}

/// Find the next resolvable shortcode at or after `from`, as
/// (start, end, emoji). `text[start..end]` is the raw shortcode.
fn next_shortcode(text: &str, from: usize) -> Option<(usize, usize, &'static str)> {
    let bytes = text.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] != b':' {
            i += 1;
            continue;
        }
        let name_start = i + 1;
        let mut j = name_start;
        while j < bytes.len() && is_name_char(bytes[j] as char) {
            j += 1;
        }
        if j == name_start || j >= bytes.len() || bytes[j] != b':' {
            i = name_start;
            continue;
        }
        let name = &text[name_start..j];
        let mut end = j + 1;
        // optional ::skin-tone-N:
        let mut emoji = None;
        if let Some(rest) = text.get(end..)
            && let Some(after) = rest.strip_prefix(":skin-tone-")
            && let Some(digit) = after.chars().next()
            && let Some(tone) = digit.to_digit(10)
            && after[1..].starts_with(':')
            && let Some(toned) = resolve_with_skin_tone(name, tone as u8)
        {
            emoji = Some(toned);
            end += ":skin-tone-".len() + 2;
        }
        match emoji.or_else(|| resolve(name)) {
            Some(e) => return Some((i, end, e)),
            // `12:30:45`: the first colon did not start a shortcode; the
            // second one may.
            None => i = j,
        }
    }
    None
}

/// Split a plain text run into text and resolved shortcodes.
pub fn segments(text: &str) -> Vec<Segment<'_>> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some((start, end, emoji)) = next_shortcode(text, pos) {
        if start > pos {
            out.push(Segment::Text(&text[pos..start]));
        }
        out.push(Segment::Emoji {
            raw: &text[start..end],
            emoji,
        });
        pos = end;
    }
    if pos < text.len() {
        out.push(Segment::Text(&text[pos..]));
    }
    out
}

/// Replace `:name:` shortcodes with emoji before sending, leaving code spans,
/// code blocks and `<:custom:123>` / `<a:custom:123>` / other `<...>` tokens
/// alone, which is what the web composer does as you type.
pub fn replace_shortcodes(text: &str) -> String {
    if !text.contains(':') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        let next_protected = [rest.find("```"), rest.find('`'), rest.find('<')]
            .into_iter()
            .flatten()
            .min();
        let (plain, protected_from) = match next_protected {
            Some(i) => (&rest[..i], i),
            None => (rest, rest.len()),
        };
        for seg in segments(plain) {
            match seg {
                Segment::Text(t) => out.push_str(t),
                Segment::Emoji { emoji, .. } => out.push_str(emoji),
            }
        }
        if protected_from >= rest.len() {
            break;
        }
        let tail = &rest[protected_from..];
        let consumed = if let Some(after) = tail.strip_prefix("```") {
            after.find("```").map(|n| n + 6).unwrap_or(tail.len())
        } else if let Some(after) = tail.strip_prefix('`') {
            after.find('`').map(|n| n + 2).unwrap_or(tail.len())
        } else {
            // `<...>`; an unclosed `<` protects only itself
            tail.find('>').map(|n| n + 1).unwrap_or(1)
        };
        out.push_str(&tail[..consumed]);
        rest = &tail[consumed..];
    }
    out
}

/// A search hit for the `:` autocomplete popup.
#[derive(Debug, Clone, Copy)]
pub struct Candidate {
    pub name: &'static str,
    pub emoji: &'static str,
}

/// Names matching `query`, best first: an exact name, then names starting
/// with the query, then names containing it. One row per emoji.
pub fn search(query: &str, limit: usize) -> Vec<Candidate> {
    let q = query.trim().to_ascii_lowercase();
    let mut scored: Vec<(u8, usize, &'static str, &'static str)> = ALL_NAMES
        .iter()
        .filter_map(|(name, emoji)| {
            let rank = if q.is_empty() {
                2
            } else if *name == q {
                0
            } else if name.starts_with(&q) {
                1
            } else if name.contains(&q) {
                2
            } else {
                return None;
            };
            Some((rank, name.len(), *name, *emoji))
        })
        .collect();
    scored.sort_unstable();
    let mut seen_emoji: Vec<&str> = Vec::new();
    let mut out = Vec::new();
    for (_, _, name, emoji) in scored {
        if seen_emoji.contains(&emoji) {
            continue;
        }
        seen_emoji.push(emoji);
        out.push(Candidate { name, emoji });
        if out.len() >= limit {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fluxer_names_resolve() {
        assert_eq!(resolve("eyes"), Some("👀"));
        assert_eq!(resolve("slight_smile"), Some("🙂"));
        assert_eq!(resolve("thumbsup"), Some("👍"));
        assert_eq!(resolve("pouting_face"), Some("😡"), "Fluxer's name wins");
        assert_eq!(resolve("EYES"), Some("👀"));
        assert_eq!(resolve("not_an_emoji_name"), None);
        assert_eq!(resolve(""), None);
    }

    #[test]
    fn skin_tones() {
        assert_eq!(
            segments(":thumbsup::skin-tone-3:"),
            vec![Segment::Emoji {
                raw: ":thumbsup::skin-tone-3:",
                emoji: "👍🏽"
            }]
        );
    }

    #[test]
    fn splits_text_and_shortcodes() {
        let segs = segments("look :eyes: at 12:30:45 :nope: ok");
        assert_eq!(
            segs,
            vec![
                Segment::Text("look "),
                Segment::Emoji {
                    raw: ":eyes:",
                    emoji: "👀"
                },
                Segment::Text(" at 12:30:45 :nope: ok"),
            ]
        );
    }

    #[test]
    fn send_time_replacement_respects_code_and_custom_emoji() {
        assert_eq!(
            replace_shortcodes("hi :eyes: `:eyes:` <:blob:123> <a:party:9> ```\n:eyes:\n``` :joy:"),
            "hi 👀 `:eyes:` <:blob:123> <a:party:9> ```\n:eyes:\n``` 😂"
        );
        assert_eq!(replace_shortcodes("no colons"), "no colons");
        assert_eq!(replace_shortcodes("unclosed ` :eyes:"), "unclosed ` :eyes:");
    }

    #[test]
    fn search_ranks_exact_name_first() {
        let hits = search("eyes", 12);
        assert_eq!(hits[0].name, "eyes");
        assert_eq!(hits[0].emoji, "👀");
        assert!(
            search("slight", 12)
                .iter()
                .any(|c| c.name == "slight_smile")
        );
        assert!(search("zzzzzz", 12).is_empty());
    }
}
