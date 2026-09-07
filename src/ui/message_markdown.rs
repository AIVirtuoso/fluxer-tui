//! Fluxer's message markup, drawn for the terminal. The syntax follows the
//! parser the web app uses (`packages/markdown_parser` in the Fluxer
//! repository), so what is shown here is what everyone else sees there.
//!
//! Blocks: `# ` to `#### ` headings, `-# ` subtext, `> ` quotes, `>>> ` for
//! the rest of the message, `> [!NOTE]` and its siblings, `- ` / `* ` /
//! `1. ` lists nested by two spaces, ``` fenced code with a language, `||`
//! spoilers over several lines, and tables. Inline: **bold**, *italic* or
//! _italic_, ***both***, __underline__, ~~strike~~, ||spoiler||, `code`,
//! \escapes, [masked](links), bare and <angle> links, <@user>, <@&role>,
//! <#channel>, </command:id>, @everyone, @here, <t:time:style>,
//! <:emoji:id> and :shortcodes:.

use crate::app::App;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use unicode_width::UnicodeWidthStr;

// ---------------------------------------------------------------------
// The two entry points.

/// A message body as rows of spans, one row per terminal line before
/// wrapping. A blank line is an empty row.
pub fn content_lines(content: &str, app: &App) -> Vec<Vec<Span<'static>>> {
    let lines: Vec<String> = content
        .split('\n')
        .map(|l| l.trim_end_matches('\r').to_string())
        .collect();
    let blocks = parse_blocks(lines, Flags::all());
    let mut out = Vec::new();
    render_blocks(&blocks, app, &[], Style::default(), &mut out);
    out
}

/// Inline markup only, for a single line such as an embed title.
pub fn parse_message_spans(text: &str, app: &App) -> Vec<Span<'static>> {
    inline(text, app, Style::default())
}

// ---------------------------------------------------------------------
// Blocks.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AlertKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Align {
    Left,
    Center,
    Right,
    None,
}

#[derive(Debug, Clone, PartialEq)]
enum Block {
    /// An empty row.
    Blank,
    /// Inline text; consecutive plain lines joined with `\n`.
    Paragraph(String),
    Heading {
        level: u8,
        text: String,
    },
    Subtext(String),
    Quote(Vec<Block>),
    Alert {
        kind: AlertKind,
        body: Vec<Block>,
    },
    List {
        ordered: bool,
        items: Vec<ListItem>,
    },
    Code {
        language: Option<String>,
        content: String,
    },
    Spoiler(Vec<Block>),
    Table {
        header: Vec<String>,
        align: Vec<Align>,
        rows: Vec<Vec<String>>,
    },
}

#[derive(Debug, Clone, PartialEq)]
struct ListItem {
    /// The item's own inline text, continuation lines joined with `\n`.
    text: String,
    /// Nested lists and code blocks under the item.
    children: Vec<Block>,
    ordinal: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
struct Flags {
    blockquotes: bool,
    multiline_blockquotes: bool,
}

impl Flags {
    fn all() -> Self {
        Self {
            blockquotes: true,
            multiline_blockquotes: true,
        }
    }
}

const MAX_LIST_DEPTH: usize = 9;

fn parse_blocks(mut lines: Vec<String>, flags: Flags) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        let line = lines[i].clone();
        let trimmed = line.trim_start();

        if trimmed.trim().is_empty() {
            let mut n = 0;
            while i + n < lines.len() && lines[i + n].trim().is_empty() {
                n += 1;
            }
            // Blank lines before the first block, after the last one, and
            // around a heading are not drawn, as in the web app.
            if !out.is_empty() && i + n < lines.len() {
                let next_is_heading = heading_of(lines[i + n].trim_start()).is_some();
                let prev_is_heading = matches!(out.last(), Some(Block::Heading { .. }));
                if !next_is_heading && !prev_is_heading {
                    out.extend(std::iter::repeat_n(Block::Blank, n));
                }
            }
            i += n;
            continue;
        }

        if flags.multiline_blockquotes && trimmed.starts_with(">>> ") {
            let mut child_lines = vec![trimmed[4..].to_string()];
            child_lines.extend(lines[i + 1..].iter().cloned());
            let inner = parse_blocks(
                child_lines,
                Flags {
                    blockquotes: true,
                    multiline_blockquotes: false,
                },
            );
            out.push(Block::Quote(inner));
            break;
        }

        if flags.blockquotes && trimmed.starts_with("> ") {
            let mut child_lines = Vec::new();
            let mut j = i;
            while j < lines.len() {
                let t = lines[j].trim_start();
                if t == "> " || t == ">  " {
                    child_lines.push(String::new());
                } else if let Some(rest) = t.strip_prefix("> ") {
                    child_lines.push(rest.to_string());
                } else {
                    break;
                }
                j += 1;
            }
            if let Some((kind, body)) = alert_of(&child_lines) {
                out.push(Block::Alert {
                    kind,
                    body: parse_blocks(
                        body,
                        Flags {
                            blockquotes: false,
                            multiline_blockquotes: false,
                        },
                    ),
                });
            } else {
                let inner = parse_blocks(
                    child_lines,
                    Flags {
                        blockquotes: false,
                        multiline_blockquotes: false,
                    },
                );
                let inner = if inner.is_empty() {
                    vec![Block::Blank]
                } else {
                    inner
                };
                out.push(Block::Quote(inner));
            }
            i = j;
            continue;
        }

        if let Some(item) = list_item_of(&line) {
            let (block, next) = parse_list(&lines, i, item.ordered, item.level, 1);
            out.push(block);
            i = next;
            continue;
        }

        if trimmed.starts_with("||") && !trimmed[2..].contains("||") {
            if let Some((inner, next, trailing)) = parse_block_spoiler(&lines, i) {
                out.push(Block::Spoiler(parse_blocks(inner, flags)));
                match trailing {
                    Some(rest) => {
                        lines[next] = rest;
                        i = next;
                    }
                    None => i = next,
                }
            } else {
                out.push(Block::Paragraph(line.clone()));
                i += 1;
            }
            continue;
        }

        if let Some(fence_pos) = find_unescaped_fence(&line) {
            let at_start = trimmed.starts_with("```") && fence_pos == line.len() - trimmed.len();
            if at_start {
                // without a closing fence the line is plain text
                if let Some(code) = parse_code_block(&lines, i) {
                    out.push(Block::Code {
                        language: code.language,
                        content: code.content,
                    });
                    match code.trailing {
                        Some(rest) => {
                            lines[code.next] = rest;
                            i = code.next;
                        }
                        None => i = code.next,
                    }
                    continue;
                }
            } else if !has_open_inline_code(&line[..fence_pos]) {
                // text before a fence on the same line: its own paragraph,
                // then the fence is looked at as the start of a line
                let prefix = line[..fence_pos].to_string();
                let rest = line[fence_pos..].to_string();
                let probe: Vec<String> = std::iter::once(rest.clone())
                    .chain(lines[i + 1..].iter().cloned())
                    .collect();
                if parse_code_block(&probe, 0).is_some() {
                    out.push(Block::Paragraph(prefix.trim_end().to_string()));
                    lines[i] = rest;
                    continue;
                }
            }
        }

        if trimmed.starts_with("-#") {
            if let Some(text) = subtext_of(trimmed) {
                out.push(Block::Subtext(text.to_string()));
            } else {
                out.push(Block::Paragraph(line.clone()));
            }
            i += 1;
            continue;
        }

        if let Some((level, text)) = heading_of(trimmed) {
            out.push(Block::Heading {
                level,
                text: text.to_string(),
            });
            i += 1;
            continue;
        }

        if trimmed.contains('|')
            && let Some((table, next)) = parse_table(&lines, i)
        {
            out.push(table);
            i = next;
            continue;
        }

        // A paragraph: this line and the plain lines after it, until a
        // block starts or a blank line.
        let mut text = line.clone();
        let mut j = i + 1;
        while j < lines.len() {
            let next = &lines[j];
            let next_trimmed = next.trim_start();
            if next_trimmed.is_empty()
                || is_block_start(next, flags)
                || is_table_start(&lines, j)
                || opens_code_block_midline(&lines, j)
            {
                break;
            }
            text.push('\n');
            text.push_str(next);
            j += 1;
        }
        out.push(Block::Paragraph(text));
        i = j;
    }
    out
}

fn is_block_start(line: &str, flags: Flags) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('#')
        || trimmed.starts_with("-#")
        || trimmed.starts_with("```")
        || list_item_of(line).is_some()
        || (flags.multiline_blockquotes && trimmed.starts_with(">>> "))
        || (flags.blockquotes && trimmed.starts_with("> "))
}

/// `#` to `####`, a space, and something visible.
fn heading_of(trimmed: &str) -> Option<(u8, &str)> {
    let level = trimmed.bytes().take(4).take_while(|b| *b == b'#').count();
    if level == 0 {
        return None;
    }
    let rest = trimmed.get(level..)?;
    let text = rest.strip_prefix(' ')?;
    if !has_visible_content(text) {
        return None;
    }
    Some((level as u8, text))
}

/// `-# ` and something visible; `-#  ` (two spaces) is not subtext.
fn subtext_of(trimmed: &str) -> Option<&str> {
    let rest = trimmed.strip_prefix("-#")?;
    let text = rest.strip_prefix(' ')?;
    if text.starts_with(' ') || !has_visible_content(text) {
        return None;
    }
    Some(text)
}

/// `[!NOTE]` and friends at the start of a quote.
fn alert_of(child_lines: &[String]) -> Option<(AlertKind, Vec<String>)> {
    let first = child_lines.first()?;
    let rest = first.strip_prefix("[!")?;
    let close = rest.find(']')?;
    let kind = match rest[..close].to_ascii_uppercase().as_str() {
        "NOTE" => AlertKind::Note,
        "TIP" => AlertKind::Tip,
        "IMPORTANT" => AlertKind::Important,
        "WARNING" => AlertKind::Warning,
        "CAUTION" => AlertKind::Caution,
        _ => return None,
    };
    let mut body = Vec::new();
    let after = rest[close + 1..].trim_start();
    if !after.is_empty() {
        body.push(after.to_string());
    }
    body.extend(child_lines[1..].iter().cloned());
    // leading and trailing blank lines are not part of the alert
    while body.first().is_some_and(|l| l.trim().is_empty()) {
        body.remove(0);
    }
    while body.last().is_some_and(|l| l.trim().is_empty()) {
        body.pop();
    }
    Some((kind, body))
}

struct ListMatch<'a> {
    ordered: bool,
    level: usize,
    content: &'a str,
    ordinal: Option<usize>,
}

/// `- `, `* ` or `1. ` after an even number of spaces (two per level).
fn list_item_of(line: &str) -> Option<ListMatch<'_>> {
    let indent = line.bytes().take_while(|b| *b == b' ').count();
    if indent == 1 || indent >= line.len() {
        return None;
    }
    let level = indent / 2;
    let rest = &line[indent..];
    if (rest.starts_with("- ") || rest.starts_with("* ")) && rest.len() > 2 {
        return Some(ListMatch {
            ordered: false,
            level,
            content: &rest[2..],
            ordinal: None,
        });
    }
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && rest[digits..].starts_with(". ") {
        return Some(ListMatch {
            ordered: true,
            level,
            content: &rest[digits + 2..],
            ordinal: rest[..digits].parse::<usize>().ok().or(Some(1)),
        });
    }
    None
}

fn parse_list(
    lines: &[String],
    start: usize,
    ordered: bool,
    level: usize,
    depth: usize,
) -> (Block, usize) {
    let mut items: Vec<ListItem> = Vec::new();
    let mut i = start;
    while i < lines.len() {
        let line = &lines[i];
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') || trimmed.starts_with("> ") || trimmed.starts_with(">>> ") {
            break;
        }
        if let Some(item) = list_item_of(line) {
            if item.level < level {
                break;
            }
            if item.level == level {
                if item.ordered != ordered {
                    break;
                }
                let ordinal = if !ordered {
                    None
                } else if let Some(first) = items.first() {
                    Some(first.ordinal.unwrap_or(1) + items.len())
                } else {
                    item.ordinal.or(Some(1))
                };
                let mut children = Vec::new();
                let text = item.content.to_string();
                i += 1;
                if i < lines.len() {
                    if let Some(code) = lines[i]
                        .trim_start()
                        .starts_with("```")
                        .then(|| parse_code_block(lines, i))
                        .flatten()
                        && code.trailing.is_none()
                    {
                        children.push(Block::Code {
                            language: code.language,
                            content: code.content,
                        });
                        i = code.next;
                    } else if let Some(nested) = list_item_of(&lines[i])
                        && nested.level > level
                        && depth < MAX_LIST_DEPTH
                    {
                        let (block, next) =
                            parse_list(lines, i, nested.ordered, nested.level, depth + 1);
                        children.push(block);
                        i = next;
                    }
                }
                items.push(ListItem {
                    text,
                    children,
                    ordinal,
                });
                continue;
            }
            if item.level == level + 1 && depth < MAX_LIST_DEPTH {
                let (block, next) = parse_list(lines, i, item.ordered, item.level, depth + 1);
                match items.last_mut() {
                    Some(last) => last.children.push(block),
                    None => items.push(ListItem {
                        text: String::new(),
                        children: vec![block],
                        ordinal: None,
                    }),
                }
                i = next;
                continue;
            }
            break;
        }
        let spaces = line.bytes().take_while(|b| *b == b' ').count();
        let continuation = spaces > level * 2;
        let bullet_text = trimmed.starts_with("- ") && !line.starts_with("  ");
        if (continuation || bullet_text) && !trimmed.is_empty() {
            if let Some(last) = items.last_mut() {
                let piece = if bullet_text { line.trim() } else { trimmed };
                if !last.text.is_empty() {
                    last.text.push('\n');
                }
                last.text.push_str(piece);
            }
            i += 1;
            continue;
        }
        break;
    }
    (Block::List { ordered, items }, i)
}

struct CodeBlock {
    language: Option<String>,
    content: String,
    /// The line after the block.
    next: usize,
    /// Text after the closing fence on its line: it becomes line `next`.
    trailing: Option<String>,
}

fn parse_code_block(lines: &[String], start: usize) -> Option<CodeBlock> {
    let line = lines.get(start)?;
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();
    let list_indent = &line[..indent];
    let fence_len = trimmed.bytes().take_while(|b| *b == b'`').count();
    if fence_len < 3 {
        return None;
    }
    let language_part = &trimmed[fence_len..];
    let closing = "`".repeat(fence_len);
    if let Some(close_at) = language_part.find(&closing) {
        // ```code``` on one line
        let inner = &language_part[..close_at];
        if !has_visible_content(inner) {
            return None;
        }
        let trailing = &language_part[close_at + fence_len..];
        return Some(CodeBlock {
            language: None,
            content: format!("{inner}\n"),
            next: if trailing.is_empty() {
                start + 1
            } else {
                start
            },
            trailing: (!trailing.is_empty()).then(|| trailing.to_string()),
        });
    }
    let language = is_code_fence_language(language_part).then(|| language_part.trim().to_string());
    // the closing fence must exist, within reason
    let mut found = false;
    for (n, candidate) in lines[start + 1..].iter().enumerate() {
        if closing_fence(candidate.trim_start(), &closing, fence_len).is_some() {
            found = true;
            break;
        }
        if n > 1000 {
            break;
        }
    }
    if !found {
        return None;
    }
    let mut content = String::new();
    if language.is_none() && !language_part.is_empty() {
        content.push_str(language_part);
        content.push('\n');
    }
    let mut i = start + 1;
    let mut trailing = None;
    while i < lines.len() {
        let current = &lines[i];
        let current_trimmed = current.trim_start();
        if let Some((fence_index, count, after)) =
            closing_fence(current_trimmed, &closing, fence_len)
        {
            let absolute = current.find(&closing).unwrap_or(0);
            let prefix = &current[..absolute];
            let content_line = if indent > 0 && prefix.starts_with(list_indent) {
                &prefix[indent..]
            } else {
                prefix
            };
            if !content_line.is_empty() {
                content.push_str(content_line);
                content.push('\n');
            }
            let extra = if !after.is_empty() {
                after
            } else if count > fence_len {
                &current_trimmed[fence_index + fence_len..]
            } else {
                ""
            };
            if !extra.is_empty() {
                if !has_visible_content(&content) {
                    return None;
                }
                trailing = Some(extra.trim_start().to_string());
                return Some(CodeBlock {
                    language,
                    content,
                    next: i,
                    trailing,
                });
            }
            i += 1;
            break;
        }
        let content_line = if indent > 0 && current.starts_with(list_indent) {
            &current[indent..]
        } else {
            current.as_str()
        };
        content.push_str(content_line);
        content.push('\n');
        i += 1;
    }
    if !has_visible_content(&content) {
        return None;
    }
    Some(CodeBlock {
        language,
        content,
        next: i,
        trailing,
    })
}

/// A closing fence on a line: where it starts, how many backticks, and
/// the text after it.
fn closing_fence<'a>(
    trimmed: &'a str,
    closing: &str,
    fence_len: usize,
) -> Option<(usize, usize, &'a str)> {
    let at = trimmed.find(closing)?;
    let count = trimmed[at..].bytes().take_while(|b| *b == b'`').count();
    let after = &trimmed[at + count..];
    let next = after.bytes().next();
    let only_ws_after = next.is_none_or(|b| matches!(b, b' ' | b'\t' | b'`'));
    if count >= fence_len && (only_ws_after || after.contains(closing)) {
        return Some((at, count, after));
    }
    None
}

fn find_unescaped_fence(line: &str) -> Option<usize> {
    let mut search = 0;
    while let Some(off) = line[search..].find("```") {
        let at = search + off;
        let backslashes = line[..at].bytes().rev().take_while(|b| *b == b'\\').count();
        if backslashes % 2 == 0 {
            return Some(at);
        }
        search = at + 1;
    }
    None
}

fn opens_code_block_midline(lines: &[String], i: usize) -> bool {
    let line = &lines[i];
    let Some(pos) = find_unescaped_fence(line) else {
        return false;
    };
    let trimmed = line.trim_start();
    if trimmed.starts_with("```") && pos == line.len() - trimmed.len() {
        return false;
    }
    if has_open_inline_code(&line[..pos]) {
        return false;
    }
    let probe: Vec<String> = std::iter::once(line[pos..].to_string())
        .chain(lines[i + 1..].iter().cloned())
        .collect();
    parse_code_block(&probe, 0).is_some()
}

/// Whether a backtick run in `text` is still open at its end.
fn has_open_inline_code(text: &str) -> bool {
    let mut open: Option<usize> = None;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'`' {
            i += 1;
            continue;
        }
        let run = bytes[i..].iter().take_while(|b| **b == b'`').count();
        match open {
            None => open = Some(run),
            Some(n) if n == run => open = None,
            Some(_) => {}
        }
        i += run;
    }
    open.is_some()
}

/// A language name after the opening fence: one word of letters, digits
/// and `_+.#/-`.
fn is_code_fence_language(part: &str) -> bool {
    if part.starts_with([' ', '\t']) {
        return false;
    }
    let trimmed = part.trim();
    if trimmed.is_empty() || trimmed.contains([' ', '\t']) {
        return false;
    }
    trimmed
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'.' | b'#' | b'/' | b'-'))
}

/// `||` opening a spoiler that closes on a later line: the lines in
/// between, the line after the closing `||`, and any text after it.
fn parse_block_spoiler(
    lines: &[String],
    start: usize,
) -> Option<(Vec<String>, usize, Option<String>)> {
    let first = &lines[start];
    let open = first.find("||")?;
    let mut inner = vec![first[open + 2..].to_string()];
    let mut i = start + 1;
    while i < lines.len() {
        let line = &lines[i];
        if let Some(end) = line.find("||") {
            inner.push(line[..end].to_string());
            let trailing = &line[end + 2..];
            let joined = inner.join("\n");
            if !has_visible_content(&joined) {
                return None;
            }
            while inner.first().is_some_and(|l| l.trim().is_empty()) {
                inner.remove(0);
            }
            while inner.last().is_some_and(|l| l.trim().is_empty()) {
                inner.pop();
            }
            return Some(if trailing.trim().is_empty() {
                (inner, i + 1, None)
            } else {
                (inner, i, Some(trailing.trim_start().to_string()))
            });
        }
        inner.push(line.clone());
        i += 1;
    }
    None
}

fn split_table_cells(line: &str) -> Vec<String> {
    let mut s = line.trim();
    if let Some(rest) = s.strip_prefix('|') {
        s = rest;
    }
    if let Some(rest) = s.strip_suffix('|') {
        s = rest;
    }
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'|') {
            cell.push('|');
            chars.next();
        } else if c == '|' {
            cells.push(std::mem::take(&mut cell));
        } else {
            cell.push(c);
        }
    }
    cells.push(cell);
    cells
}

fn table_alignments(cells: &[String]) -> Option<Vec<Align>> {
    if cells.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(cells.len());
    for cell in cells {
        let v = cell.trim();
        if v.is_empty()
            || !v.contains('-')
            || !v.bytes().all(|b| matches!(b, b' ' | b':' | b'-' | b'|'))
        {
            return None;
        }
        let left = v.starts_with(':');
        let right = v.ends_with(':');
        out.push(match (left, right) {
            (true, true) => Align::Center,
            (true, false) => Align::Left,
            (false, true) => Align::Right,
            (false, false) => Align::None,
        });
    }
    Some(out)
}

fn is_table_block_break(text: &str) -> bool {
    let t = text.trim();
    let Some(first) = t.bytes().next() else {
        return false;
    };
    if matches!(first, b'#' | b'>' | b'-' | b'*') {
        return true;
    }
    first.is_ascii_digit() && t[1..t.len().min(4)].contains('.')
}

fn table_head(lines: &[String], i: usize) -> Option<(Vec<String>, Vec<Align>)> {
    if i + 2 >= lines.len() {
        return None;
    }
    let header = lines[i].trim();
    let align = lines[i + 1].trim();
    if !header.contains('|') || !align.contains('|') {
        return None;
    }
    let header_cells = split_table_cells(header);
    if header_cells.iter().all(|c| c.trim().is_empty()) {
        return None;
    }
    let alignments = table_alignments(&split_table_cells(align))?;
    if header_cells.len() != alignments.len() {
        return None;
    }
    Some((header_cells, alignments))
}

fn is_table_start(lines: &[String], i: usize) -> bool {
    let Some(_) = table_head(lines, i) else {
        return false;
    };
    let body = lines[i + 2].trim();
    body.contains('|') && !is_table_block_break(body)
}

fn parse_table(lines: &[String], i: usize) -> Option<(Block, usize)> {
    let (header, align) = table_head(lines, i)?;
    let columns = header.len();
    let mut rows = Vec::new();
    let mut j = i + 2;
    while j < lines.len() {
        let line = lines[j].trim();
        if !line.contains('|') || is_table_block_break(line) {
            break;
        }
        let mut cells = split_table_cells(line);
        if cells.len() > columns {
            let extra = cells.split_off(columns);
            let last = cells.last_mut().expect("columns > 0");
            for e in extra {
                last.push('|');
                last.push_str(&e);
            }
        }
        cells.resize(columns, String::new());
        rows.push(cells);
        j += 1;
    }
    if rows.is_empty() {
        return None;
    }
    Some((
        Block::Table {
            header,
            align,
            rows,
        },
        j,
    ))
}

// ---------------------------------------------------------------------
// Rendering blocks to rows.

fn render_blocks(
    blocks: &[Block],
    app: &App,
    prefix: &[Span<'static>],
    base: Style,
    out: &mut Vec<Vec<Span<'static>>>,
) {
    for block in blocks {
        match block {
            Block::Blank => out.push(prefix.to_vec()),
            Block::Paragraph(text) => push_rows(out, prefix, inline(text, app, base)),
            Block::Heading { level, text } => {
                let mut row = prefix.to_vec();
                row.push(Span::styled(
                    format!("{} ", "#".repeat(*level as usize)),
                    base.patch(
                        Style::default()
                            .fg(crate::ui::theme::accent())
                            .add_modifier(Modifier::BOLD),
                    ),
                ));
                row.extend(inline(text, app, base.add_modifier(Modifier::BOLD)));
                out.push(row);
            }
            Block::Subtext(text) => push_rows(
                out,
                prefix,
                inline(text, app, base.patch(crate::ui::theme::muted_style())),
            ),
            Block::Quote(inner) => {
                let mut bar = prefix.to_vec();
                bar.push(Span::styled(
                    "\u{258E} ",
                    base.patch(crate::ui::theme::muted_style()),
                ));
                render_blocks(inner, app, &bar, base, out);
            }
            Block::Alert { kind, body } => {
                let colour = alert_colour(*kind);
                let mut bar = prefix.to_vec();
                bar.push(Span::styled("\u{258E} ", base.fg(colour)));
                let mut title = bar.clone();
                title.push(Span::styled(
                    format!(" {} ", alert_label(*kind)),
                    base.patch(crate::ui::theme::pill_style(colour))
                        .add_modifier(Modifier::BOLD),
                ));
                out.push(title);
                render_blocks(body, app, &bar, base, out);
            }
            Block::List { ordered, items } => {
                render_list(*ordered, items, app, prefix, base, 0, out)
            }
            Block::Code { language, content } => {
                let bar = Span::styled("\u{2503} ", base.patch(crate::ui::theme::muted_style()));
                if let Some(language) = language {
                    let mut row = prefix.to_vec();
                    row.push(bar.clone());
                    row.push(Span::styled(
                        language.clone(),
                        base.patch(crate::ui::theme::dim_style())
                            .add_modifier(Modifier::ITALIC),
                    ));
                    out.push(row);
                }
                for line in content.lines() {
                    let mut row = prefix.to_vec();
                    row.push(bar.clone());
                    row.push(Span::styled(
                        line.replace('\t', "    "),
                        base.patch(crate::ui::theme::code_style()),
                    ));
                    out.push(row);
                }
            }
            Block::Spoiler(inner) => {
                render_blocks(inner, app, prefix, base.patch(spoiler_style()), out)
            }
            Block::Table {
                header,
                align,
                rows,
            } => render_table(header, align, rows, app, prefix, base, out),
        }
    }
}

/// Rows out of inline spans: split at `\n`, the prefix in front of each.
fn push_rows(
    out: &mut Vec<Vec<Span<'static>>>,
    prefix: &[Span<'static>],
    spans: Vec<Span<'static>>,
) {
    for row in split_rows(spans) {
        let mut line = prefix.to_vec();
        line.extend(row);
        out.push(line);
    }
}

fn split_rows(spans: Vec<Span<'static>>) -> Vec<Vec<Span<'static>>> {
    let mut rows: Vec<Vec<Span<'static>>> = vec![Vec::new()];
    for span in spans {
        if !span.content.contains('\n') {
            rows.last_mut().expect("one row").push(span);
            continue;
        }
        let style = span.style;
        for (n, piece) in span.content.split('\n').enumerate() {
            if n > 0 {
                rows.push(Vec::new());
            }
            if !piece.is_empty() {
                rows.last_mut()
                    .expect("one row")
                    .push(Span::styled(piece.to_string(), style));
            }
        }
    }
    rows
}

fn render_list(
    ordered: bool,
    items: &[ListItem],
    app: &App,
    prefix: &[Span<'static>],
    base: Style,
    depth: usize,
    out: &mut Vec<Vec<Span<'static>>>,
) {
    const BULLETS: [&str; 3] = ["\u{2022} ", "\u{25E6} ", "\u{25AA} "];
    let indent = "  ".repeat(depth);
    for (n, item) in items.iter().enumerate() {
        let marker = if ordered {
            format!("{}. ", item.ordinal.unwrap_or(n + 1))
        } else {
            BULLETS[depth % BULLETS.len()].to_string()
        };
        let hang = " ".repeat(UnicodeWidthStr::width(marker.as_str()));
        let rows = split_rows(inline(&item.text, app, base));
        for (r, row) in rows.into_iter().enumerate() {
            let mut line = prefix.to_vec();
            if r == 0 {
                line.push(Span::styled(format!("{indent}{marker}"), base));
            } else {
                line.push(Span::styled(format!("{indent}{hang}"), base));
            }
            line.extend(row);
            out.push(line);
        }
        for child in &item.children {
            match child {
                Block::List {
                    ordered: nested_ordered,
                    items: nested,
                } => render_list(*nested_ordered, nested, app, prefix, base, depth + 1, out),
                other => {
                    let mut inner_prefix = prefix.to_vec();
                    inner_prefix.push(Span::styled(format!("{indent}{hang}"), base));
                    render_blocks(std::slice::from_ref(other), app, &inner_prefix, base, out);
                }
            }
        }
    }
}

fn spans_width(spans: &[Span<'static>]) -> usize {
    spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum()
}

fn render_table(
    header: &[String],
    align: &[Align],
    rows: &[Vec<String>],
    app: &App,
    prefix: &[Span<'static>],
    base: Style,
    out: &mut Vec<Vec<Span<'static>>>,
) {
    let head: Vec<Vec<Span<'static>>> = header
        .iter()
        .map(|c| inline(c.trim(), app, base.add_modifier(Modifier::BOLD)))
        .collect();
    let body: Vec<Vec<Vec<Span<'static>>>> = rows
        .iter()
        .map(|row| row.iter().map(|c| inline(c.trim(), app, base)).collect())
        .collect();
    let columns = header.len();
    let mut widths = vec![1usize; columns];
    for (c, cell) in head.iter().enumerate() {
        widths[c] = widths[c].max(spans_width(cell));
    }
    for row in &body {
        for (c, cell) in row.iter().enumerate() {
            widths[c] = widths[c].max(spans_width(cell));
        }
    }
    let rule = base.patch(crate::ui::theme::muted_style());
    let separator = Span::styled(" \u{2502} ", rule);
    let table_row = |cells: &[Vec<Span<'static>>]| -> Vec<Span<'static>> {
        let mut line = prefix.to_vec();
        for (c, cell) in cells.iter().enumerate() {
            if c > 0 {
                line.push(separator.clone());
            }
            let pad = widths[c].saturating_sub(spans_width(cell));
            let (left, right) = match align.get(c).copied().unwrap_or(Align::None) {
                Align::Right => (pad, 0),
                Align::Center => (pad / 2, pad - pad / 2),
                Align::Left | Align::None => (0, pad),
            };
            if left > 0 {
                line.push(Span::styled(" ".repeat(left), base));
            }
            line.extend(cell.iter().cloned());
            if right > 0 {
                line.push(Span::styled(" ".repeat(right), base));
            }
        }
        line
    };
    out.push(table_row(&head));
    let mut line = prefix.to_vec();
    let dashes: Vec<String> = widths.iter().map(|w| "\u{2500}".repeat(*w)).collect();
    line.push(Span::styled(dashes.join("\u{2500}\u{253C}\u{2500}"), rule));
    out.push(line);
    for row in &body {
        out.push(table_row(row));
    }
}

fn alert_label(kind: AlertKind) -> &'static str {
    match kind {
        AlertKind::Note => "NOTE",
        AlertKind::Tip => "TIP",
        AlertKind::Important => "IMPORTANT",
        AlertKind::Warning => "WARNING",
        AlertKind::Caution => "CAUTION",
    }
}

fn alert_colour(kind: AlertKind) -> Color {
    let terminal = crate::ui::theme::is_terminal_theme();
    match kind {
        AlertKind::Note => crate::ui::theme::accent(),
        AlertKind::Tip => crate::ui::theme::voice_color(),
        AlertKind::Important => {
            if terminal {
                Color::Magenta
            } else {
                Color::Rgb(163, 113, 247)
            }
        }
        AlertKind::Warning => {
            if terminal {
                Color::Yellow
            } else {
                Color::Rgb(250, 166, 26)
            }
        }
        AlertKind::Caution => crate::ui::theme::danger(),
    }
}

fn spoiler_style() -> Style {
    crate::ui::theme::muted_style().add_modifier(Modifier::DIM)
}

// ---------------------------------------------------------------------
// Inline markup.

const MAX_INLINE_DEPTH: usize = 10;

/// The delimiters already open around the text being parsed: the same
/// one does not open again inside itself.
#[derive(Clone, Default)]
struct Ctx {
    active: Vec<(u8, bool)>,
}

impl Ctx {
    fn can_enter(&self, first: u8, double: bool) -> bool {
        self.active.len() < MAX_INLINE_DEPTH && !self.active.contains(&(first, double))
    }

    fn with(&self, first: u8, double: bool) -> Ctx {
        let mut next = self.clone();
        next.active.push((first, double));
        next
    }
}

fn inline(text: &str, app: &App, base: Style) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    parse_inline(text, app, base, &Ctx::default(), &mut out);
    out
}

fn byte_at(s: &str, i: usize) -> u8 {
    s.as_bytes().get(i).copied().unwrap_or(0)
}

fn advance_one(s: &str, i: usize) -> usize {
    if i >= s.len() {
        0
    } else if s.is_char_boundary(i) {
        s[i..].chars().next().map_or(1, char::len_utf8)
    } else {
        1
    }
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

fn is_punct(b: u8) -> bool {
    b.is_ascii_punctuation()
}

fn is_alnum(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

fn is_formatting_char(b: u8) -> bool {
    matches!(b, b'*' | b'_' | b'~' | b'|' | b'`')
}

fn is_escapable(b: u8) -> bool {
    matches!(
        b,
        b'[' | b']'
            | b'('
            | b')'
            | b'\\'
            | b'*'
            | b'_'
            | b'~'
            | b'`'
            | b'@'
            | b'#'
            | b'-'
            | b'|'
            | b':'
            | b'<'
            | b'>'
    )
}

fn has_visible_content(s: &str) -> bool {
    s.chars().any(|c| {
        !c.is_whitespace()
            && !matches!(
                c,
                '\0' | '\u{00ad}' | '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{2060}' | '\u{feff}'
            )
    })
}

fn parse_inline(text: &str, app: &App, base: Style, ctx: &Ctx, out: &mut Vec<Span<'static>>) {
    let mut buf = String::new();
    let mut pos = 0usize;
    while pos < text.len() {
        let current = byte_at(text, pos);
        let rest = &text[pos..];

        if current == b'\\' && pos + 1 < text.len() {
            let next = byte_at(text, pos + 1);
            if next == b':'
                && let Some(end) = shortcode_end(&text[pos + 1..])
            {
                // an escaped shortcode stays text
                flush(&mut buf, base, out);
                out.push(Span::styled(text[pos + 1..pos + 1 + end].to_string(), base));
                pos += 1 + end;
                continue;
            }
            if next == b'`' {
                let run = text[pos + 1..].bytes().take_while(|b| *b == b'`').count();
                if run >= 3 {
                    buf.push_str(&text[pos + 1..pos + 1 + run]);
                    pos += 1 + run;
                    continue;
                }
            }
            if is_escapable(next) || (next == b'.' && is_dot_escape(text, pos)) {
                buf.push(next as char);
                pos += 2;
                continue;
            }
        }

        if starts_with_url(rest)
            && !buf.ends_with("<\"")
            && !buf.ends_with("<'")
            && let Some(url) = url_segment(rest)
        {
            flush(&mut buf, base, out);
            out.push(link_span(url, base));
            pos += url.len();
            continue;
        }

        if current == b'_' && byte_at(text, pos + 1) != b'_' && is_word_underscore(text, pos) {
            buf.push('_');
            pos += 1;
            continue;
        }

        if current == b'<' {
            if let Some(n) = custom_emoji(rest, app, base, out, &mut buf) {
                pos += n;
                continue;
            }
            if rest.starts_with("<t:")
                && let Some((label, n)) = timestamp(rest, app.ui_settings.clock_12h)
            {
                flush(&mut buf, base, out);
                out.push(Span::styled(
                    label,
                    base.patch(crate::ui::theme::dim_style()),
                ));
                pos += n;
                continue;
            }
            if let Some((span, n)) = mention(rest, app, base) {
                flush(&mut buf, base, out);
                out.push(span);
                pos += n;
                continue;
            }
            if let Some((span, n)) = angle_link(rest, base) {
                flush(&mut buf, base, out);
                out.push(span);
                pos += n;
                continue;
            }
        }

        if current == b'@' && (pos == 0 || byte_at(text, pos - 1) != b'\\') {
            let word = if rest.starts_with("@everyone") {
                Some("@everyone")
            } else if rest.starts_with("@here") {
                Some("@here")
            } else {
                None
            };
            if let Some(word) = word {
                flush(&mut buf, base, out);
                out.push(Span::styled(
                    word.to_string(),
                    base.patch(
                        Style::default()
                            .fg(crate::ui::theme::accent())
                            .add_modifier(Modifier::BOLD),
                    ),
                ));
                pos += word.len();
                continue;
            }
        }

        if current >= 0x80 {
            let n = advance_one(text, pos);
            buf.push_str(&text[pos..pos + n]);
            pos += n;
            continue;
        }

        let double_underscore = current == b'_' && byte_at(text, pos + 1) == b'_';
        let after_word = current == b'_' && buf.bytes().next_back().is_some_and(is_alnum);
        if (is_formatting_char(current) || current == b'[') && (double_underscore || !after_word) {
            let previous = if pos > 0 { byte_at(text, pos - 1) } else { 0 };
            let consumed = if current == b'[' {
                masked_link(rest, app, base, ctx, out, &mut buf)
            } else {
                formatting(rest, previous, app, base, ctx, out, &mut buf)
            };
            if let Some(n) = consumed {
                pos += n;
                continue;
            }
        }

        buf.push(current as char);
        pos += 1;
    }
    flush(&mut buf, base, out);
}

/// `\.` after a number at the start of a line (so it is not a list) or
/// after a word.
fn is_dot_escape(text: &str, backslash: usize) -> bool {
    if backslash == 0 {
        return false;
    }
    let previous = byte_at(text, backslash - 1);
    if previous.is_ascii_alphanumeric() || previous == b'.' {
        return true;
    }
    false
}

fn is_word_underscore(text: &str, pos: usize) -> bool {
    let prev = if pos > 0 { byte_at(text, pos - 1) } else { 0 };
    let next = byte_at(text, pos + 1);
    (is_alnum(prev) || prev == b'_') && (is_alnum(next) || next == b'_')
}

/// Text collected so far, as spans: `:shortcodes:` become the emoji, and
/// emoji get the emoji colour.
fn flush(buf: &mut String, base: Style, out: &mut Vec<Span<'static>>) {
    if buf.is_empty() {
        return;
    }
    let text = std::mem::take(buf);
    emit_text(&text, base, out);
}

fn emit_text(text: &str, base: Style, out: &mut Vec<Span<'static>>) {
    if text.is_empty() {
        return;
    }
    if text.contains(':') {
        for seg in crate::emoji::segments(text) {
            match seg {
                crate::emoji::Segment::Text(t) => emit_with_emoji(t, base, out),
                crate::emoji::Segment::Emoji { emoji, .. } => out.push(Span::styled(
                    emoji.to_string(),
                    base.fg(crate::ui::theme::emoji_unknown()),
                )),
            }
        }
        return;
    }
    emit_with_emoji(text, base, out);
}

fn emit_with_emoji(text: &str, base: Style, out: &mut Vec<Span<'static>>) {
    let mut plain_from = 0usize;
    let mut i = 0usize;
    while i < text.len() {
        if byte_at(text, i) < 0x80 {
            i += 1;
            continue;
        }
        if let Some(len) = emoji_len_at(text, i) {
            if plain_from < i {
                out.push(Span::styled(text[plain_from..i].to_string(), base));
            }
            out.push(Span::styled(
                text[i..i + len].to_string(),
                base.fg(crate::ui::theme::emoji_unknown()),
            ));
            i += len;
            plain_from = i;
            continue;
        }
        i += advance_one(text, i);
    }
    if plain_from < text.len() {
        out.push(Span::styled(text[plain_from..].to_string(), base));
    }
}

/// The longest emoji the `emojis` crate knows starting at `i`.
fn emoji_len_at(text: &str, i: usize) -> Option<usize> {
    let ends: Vec<usize> = text[i..]
        .char_indices()
        .map(|(k, c)| i + k + c.len_utf8())
        .take(8)
        .collect();
    ends.into_iter()
        .rev()
        .find(|end| emojis::get(&text[i..*end]).is_some())
        .map(|end| end - i)
}

/// Length of a `:name:` (with an optional `:skin-tone-N:`) at the start
/// of `text`, if the name is known.
fn shortcode_end(text: &str) -> Option<usize> {
    match crate::emoji::segments(text).first()? {
        crate::emoji::Segment::Emoji { raw, .. } => Some(raw.len()),
        crate::emoji::Segment::Text(_) => None,
    }
}

// ----- links

fn starts_with_url(text: &str) -> bool {
    text.starts_with("http://") || text.starts_with("https://") || starts_with_app_url(text)
}

fn starts_with_app_url(text: &str) -> bool {
    text.strip_prefix("fluxer:")
        .and_then(|rest| rest.bytes().next())
        .is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'-'))
}

fn url_prefix_len(text: &str) -> Option<usize> {
    if text.starts_with("https://") {
        Some(8)
    } else if text.starts_with("http://") || starts_with_app_url(text) {
        // "http://" and "fluxer:" are both seven bytes long
        Some(7)
    } else {
        None
    }
}

fn is_url_termination(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b')' | b'"')
}

fn has_terminal_tld(text: &str) -> bool {
    let letters = text
        .bytes()
        .rev()
        .take_while(u8::is_ascii_alphabetic)
        .count();
    letters >= 2 && text.len() > letters && byte_at(text, text.len() - letters - 1) == b'.'
}

/// A bare URL at the start of `text`, as far as it goes.
fn url_segment(text: &str) -> Option<&str> {
    let prefix_len = url_prefix_len(text)?;
    let mut end = prefix_len;
    let mut depth = 0usize;
    while end < text.len() {
        let c = byte_at(text, end);
        if c == b'(' {
            depth += 1;
            end += 1;
        } else if c == b')' {
            if depth == 0 {
                break;
            }
            depth -= 1;
            end += 1;
        } else if is_url_termination(c) {
            break;
        } else {
            end += advance_one(text, end);
        }
    }
    while end > prefix_len
        && matches!(
            byte_at(text, end - 1),
            b'.' | b',' | b';' | b':' | b'!' | b'?'
        )
        && !has_terminal_tld(&text[..end])
    {
        end -= 1;
    }
    if end <= prefix_len {
        return None;
    }
    let url = &text[..end];
    let authority_end = url[prefix_len..]
        .find(['/', '?', '#'])
        .map_or(url.len(), |k| prefix_len + k);
    let authority = &url[prefix_len..authority_end];
    if authority.is_empty()
        || authority
            .bytes()
            .any(|b| matches!(b, b'*' | b'~' | b'|' | b'`' | b'<' | b'>'))
        || url[prefix_len..].contains("http://")
        || url[prefix_len..].contains("https://")
        || url.contains('<')
        || url.contains('>')
    {
        return None;
    }
    Some(url)
}

fn link_span(url: &str, base: Style) -> Span<'static> {
    Span::styled(
        url.to_string(),
        base.patch(Style::default().fg(crate::ui::theme::link_color())),
    )
}

/// `<https://...>`, `<user@host>` and `<+phone>`.
fn angle_link(text: &str, base: Style) -> Option<(Span<'static>, usize)> {
    let end = text.find('>')?;
    let inner = &text[1..end];
    if inner.is_empty() || matches!(byte_at(inner, 0), b'"' | b'\'') {
        return None;
    }
    if starts_with_url(inner) {
        let prefix_len = url_prefix_len(inner)?;
        if inner.len() <= prefix_len || inner.contains(char::is_whitespace) {
            return None;
        }
        return Some((link_span(inner, base), end + 1));
    }
    if inner.starts_with('+')
        && inner.len() > 3
        && inner[1..]
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b' ' | b'-'))
    {
        return Some((link_span(inner, base), end + 1));
    }
    if is_email(inner) {
        return Some((link_span(inner, base), end + 1));
    }
    None
}

fn is_email(text: &str) -> bool {
    let Some(at) = text.find('@') else {
        return false;
    };
    let (user, host) = (&text[..at], &text[at + 1..]);
    !user.is_empty()
        && !host.is_empty()
        && host.contains('.')
        && !host.starts_with('.')
        && !host.ends_with('.')
        && text.bytes().all(|b| b.is_ascii_graphic())
        && !text[at + 1..].contains('@')
}

/// `[label](url)`: the link's spans go to `out` and its length comes
/// back; text that only looks like one is copied to `buf` so it is not
/// parsed twice. `None` leaves the `[` to the caller.
fn masked_link(
    text: &str,
    app: &App,
    base: Style,
    ctx: &Ctx,
    out: &mut Vec<Span<'static>>,
    buf: &mut String,
) -> Option<usize> {
    let close = closing_bracket(text)?;
    let label = &text[1..close];
    if byte_at(text, close + 1) != b'(' {
        return None;
    }
    let (url, escaped, advance) = extract_url(text, close + 2)?;
    let label_trimmed = label.trim();
    let label_is_link = label.contains("](") || label.contains("<http") || label.contains('[');
    let suspicious = starts_with_url(label_trimmed) && !same_link(label_trimmed, url);
    if label_is_link
        || !has_visible_content(label)
        || is_email(label_trimmed)
        || url.len() > 2048
        || !is_valid_link_url(url)
        || suspicious
    {
        buf.push_str(&text[..advance]);
        return Some(advance);
    }
    let _ = escaped;
    flush(buf, base, out);
    let mut label_spans = Vec::new();
    let link_style = base.patch(
        Style::default()
            .fg(crate::ui::theme::link_color())
            .add_modifier(Modifier::UNDERLINED),
    );
    parse_inline(label, app, link_style, ctx, &mut label_spans);
    out.extend(label_spans);
    if !starts_with_url(label_trimmed)
        && let Some(host) = link_host(url)
    {
        out.push(Span::styled(
            format!(" ({host})"),
            base.patch(crate::ui::theme::dim_style()),
        ));
    }
    Some(advance)
}

fn closing_bracket(text: &str) -> Option<usize> {
    let mut pos = 1;
    let mut nested = 0usize;
    while pos < text.len() {
        match byte_at(text, pos) {
            b'[' => {
                nested += 1;
                pos += 1;
            }
            b']' if nested > 0 => {
                nested -= 1;
                pos += 1;
            }
            b']' => return Some(pos),
            b'\\' => pos += if pos + 1 < text.len() { 2 } else { 1 },
            _ => pos += advance_one(text, pos),
        }
        if pos > 2048 {
            break;
        }
    }
    None
}

/// The URL in `(...)` starting at `start`: the URL, whether it was
/// written as `<url>`, and the position after the `)`.
fn extract_url(text: &str, start: usize) -> Option<(&str, bool, usize)> {
    if start >= text.len() {
        return None;
    }
    if byte_at(text, start) == b'<' {
        let close = text[start + 1..].find('>')? + start + 1;
        if byte_at(text, close + 1) != b')' {
            return None;
        }
        return Some((&text[start + 1..close], true, close + 2));
    }
    let mut pos = start;
    let mut nested = 0usize;
    while pos < text.len() {
        match byte_at(text, pos) {
            b'(' => {
                nested += 1;
                pos += 1;
            }
            b')' if nested > 0 => {
                nested -= 1;
                pos += 1;
            }
            b')' => return Some((&text[start..pos], false, pos + 1)),
            _ => pos += advance_one(text, pos),
        }
    }
    None
}

fn is_valid_link_url(url: &str) -> bool {
    let Some(prefix_len) = url_prefix_len(url) else {
        return false;
    };
    url.len() > prefix_len
        && !url.contains(char::is_whitespace)
        && !url.contains(['<', '>'])
        && !url[prefix_len..].contains("http://")
        && !url[prefix_len..].contains("https://")
}

fn link_host(url: &str) -> Option<&str> {
    let prefix_len = url_prefix_len(url)?;
    let rest = &url[prefix_len..];
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = rest[..end].rsplit('@').next().unwrap_or("");
    (!host.is_empty()).then_some(host)
}

/// Whether a label that is itself a URL points where the link goes.
fn same_link(label: &str, url: &str) -> bool {
    let norm = |s: &str| {
        let s = s.trim_end_matches('/');
        let (scheme, rest) = s.split_once("://").unwrap_or(("", s));
        let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
        format!(
            "{}://{}/{}",
            scheme.to_lowercase(),
            host.to_lowercase(),
            path
        )
    };
    norm(label) == norm(url)
}

// ----- mentions, emoji, timestamps

fn custom_emoji(
    text: &str,
    app: &App,
    base: Style,
    out: &mut Vec<Span<'static>>,
    buf: &mut String,
) -> Option<usize> {
    let animated = text.starts_with("<a:");
    if !animated && !text.starts_with("<:") {
        return None;
    }
    let end = text.find('>')?;
    let inner = &text[if animated { 3 } else { 2 }..end];
    let (name, id) = inner.split_once(':')?;
    if name.is_empty()
        || id.is_empty()
        || !name
            .bytes()
            .all(|b| is_alnum(b) || matches!(b, b'_' | b'-'))
        || !id.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    flush(buf, base, out);
    match app.custom_emoji_placeholder(id, animated) {
        Some(picture) => out.push(picture),
        None => {
            let style = if animated {
                Style::default()
                    .fg(crate::ui::theme::accent())
                    .add_modifier(Modifier::ITALIC)
            } else {
                Style::default().fg(crate::ui::theme::emoji_unknown())
            };
            out.push(Span::styled(format!(":{name}:"), base.patch(style)));
        }
    }
    Some(end + 1)
}

/// `<t:unix>` and `<t:unix:style>`, in local time.
fn timestamp(text: &str, clock_12h: bool) -> Option<(String, usize)> {
    let end = text.find('>')?;
    let inner = &text[3..end];
    let mut parts = inner.split(':');
    let unix = parts.next()?;
    let style = parts.next();
    if parts.next().is_some() || unix.is_empty() || !unix.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let unix: i64 = unix.parse().ok()?;
    if unix == 0 || unix > 8_640_000_000_000 {
        return None;
    }
    let style = match style {
        None => 'f',
        Some(s) if s.len() == 1 && "tTdDfFsSR".contains(s) => s.chars().next()?,
        Some(_) => return None,
    };
    let when = chrono::DateTime::from_timestamp(unix, 0)?;
    let local = when.with_timezone(&chrono::Local);
    let time = if clock_12h { "%-I:%M %p" } else { "%H:%M" };
    let time_s = if clock_12h {
        "%-I:%M:%S %p"
    } else {
        "%H:%M:%S"
    };
    let label = match style {
        't' => local.format(time).to_string(),
        'T' => local.format(time_s).to_string(),
        'd' => local.format("%Y-%m-%d").to_string(),
        'D' => local.format("%B %-d, %Y").to_string(),
        'F' => local.format(&format!("%A, %B %-d, %Y {time}")).to_string(),
        's' => local.format(&format!("%Y-%m-%d {time}")).to_string(),
        'S' => local.format(&format!("%Y-%m-%d {time_s}")).to_string(),
        'R' => relative_time(unix, chrono::Utc::now().timestamp()),
        _ => local.format(&format!("%B %-d, %Y {time}")).to_string(),
    };
    Some((label, end + 1))
}

fn rounded(secs: u64, unit: u64) -> u64 {
    (secs + unit / 2) / unit
}

fn relative_time(unix: i64, now: i64) -> String {
    let diff = unix - now;
    let ago = diff < 0;
    let secs = diff.unsigned_abs();
    let (n, unit) = if secs < 45 {
        return if ago {
            "just now".to_string()
        } else {
            "in a moment".to_string()
        };
    } else if secs < 90 {
        (1, "minute")
    } else if secs < 45 * 60 {
        (rounded(secs, 60), "minutes")
    } else if secs < 90 * 60 {
        (1, "hour")
    } else if secs < 22 * 3600 {
        (rounded(secs, 3600), "hours")
    } else if secs < 36 * 3600 {
        (1, "day")
    } else if secs < 26 * 86400 {
        (rounded(secs, 86400), "days")
    } else if secs < 46 * 86400 {
        (1, "month")
    } else if secs < 320 * 86400 {
        (rounded(secs, 30 * 86400), "months")
    } else if secs < 548 * 86400 {
        (1, "year")
    } else {
        (rounded(secs, 365 * 86400), "years")
    };
    let amount = if n == 1 {
        if unit.starts_with('h') { "an" } else { "a" }.to_string()
    } else {
        n.to_string()
    };
    if ago {
        format!("{amount} {unit} ago")
    } else {
        format!("in {amount} {unit}")
    }
}

/// `<@id>`, `<@!id>`, `<@&id>`, `<#id>`, `</command:id>`, `<id:...>`.
fn mention(text: &str, app: &App, base: Style) -> Option<(Span<'static>, usize)> {
    let end = text.find('>')?;
    let inner = &text[1..end];
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let span = match byte_at(inner, 0) {
        b'@' if inner.starts_with("@&") => {
            let id = &inner[2..];
            if !digits(id) {
                return None;
            }
            let role = role_mention_span(app, id);
            Span::styled(role.content.to_string(), base.patch(role.style))
        }
        b'@' => {
            let id = inner.strip_prefix("@!").unwrap_or(&inner[1..]);
            if !digits(id) {
                return None;
            }
            let name = resolve_user_name(app, id);
            let is_self = id == app.me.id;
            let g = app.guild_id_for_active_channel();
            let fg = app.member_name_color(g.as_deref(), id, is_self);
            Span::styled(format!("@{name}"), base.patch(Style::default().fg(fg)))
        }
        b'#' => {
            let id = &inner[1..];
            if !digits(id) {
                return None;
            }
            let name = resolve_channel_name(app, id);
            Span::styled(
                format!("#{name}"),
                base.patch(Style::default().fg(crate::ui::theme::link_color())),
            )
        }
        b'/' => {
            let (command, id) = inner.rsplit_once(':')?;
            if !digits(id) || command.len() < 2 {
                return None;
            }
            let words: Vec<&str> = command[1..].split(' ').collect();
            if words.len() > 3
                || words.iter().any(|w| {
                    w.is_empty() || !w.bytes().all(|b| is_alnum(b) || b == b'_' || b == b'-')
                })
            {
                return None;
            }
            Span::styled(
                format!("/{}", words.join(" ")),
                base.patch(
                    Style::default()
                        .fg(crate::ui::theme::accent())
                        .add_modifier(Modifier::BOLD),
                ),
            )
        }
        b'i' if inner.starts_with("id:") => {
            let mut parts = inner.split(':');
            parts.next();
            let label = match parts.next()? {
                "customize" => "Channels & Roles",
                "browse" => "Browse Channels",
                "guide" => "Community Guide",
                "linked-roles" => "Linked Roles",
                _ => return None,
            };
            Span::styled(
                label.to_string(),
                base.patch(Style::default().fg(crate::ui::theme::link_color())),
            )
        }
        _ => return None,
    };
    Some((span, end + 1))
}

fn role_mention_span(app: &App, role_id: &str) -> Span<'static> {
    let tail = role_id
        .len()
        .checked_sub(4)
        .map(|i| &role_id[i..])
        .unwrap_or(role_id);
    let fallback = Style::default()
        .fg(crate::ui::theme::accent())
        .add_modifier(Modifier::BOLD);

    let Some(gid) = app.guild_id_for_active_channel() else {
        return Span::styled(format!("@role-{tail}"), fallback);
    };
    let Some(roles) = app.guild_roles.get(&gid) else {
        return Span::styled(
            format!("@role-{tail}"),
            fallback.add_modifier(Modifier::UNDERLINED),
        );
    };
    let Some(role) = roles.iter().find(|r| r.id.trim() == role_id.trim()) else {
        return Span::styled(format!("@role-{tail}"), fallback);
    };
    let name = if role.name.trim().is_empty() {
        format!("role-{tail}")
    } else {
        role.name.clone()
    };
    Span::styled(
        format!("@{name}"),
        crate::ui::theme::role_mention_style(role.color).add_modifier(Modifier::UNDERLINED),
    )
}

fn resolve_channel_name(app: &App, id: &str) -> String {
    for channels in app.guild_channels.values() {
        if let Some(ch) = channels.iter().find(|c| c.id == id) {
            return ch.name.clone();
        }
    }
    for ch in &app.private_channels {
        if ch.id == id {
            return ch.name.clone();
        }
    }
    format!("unknown-{}", &id[id.len().saturating_sub(4)..])
}

fn resolve_user_name(app: &App, id: &str) -> String {
    let active_gid = app.guild_id_for_active_channel();
    if let Some(gid) = active_gid.as_deref() {
        if let Some(members) = app.guild_members.get(gid)
            && let Some(m) = members.iter().find(|m| m.user.id == id)
        {
            let u = app.user_cache.get(id).unwrap_or(&m.user);
            return app.shown_name_for_user(Some(gid), u);
        }
        if let Some(u) = app.user_cache.get(id) {
            return app.shown_name_for_user(Some(gid), u);
        }
    }
    if let Some(u) = app.user_cache.get(id) {
        return app.shown_name_for_user(None, u);
    }
    for (gid, members) in &app.guild_members {
        if let Some(m) = members.iter().find(|m| m.user.id == id) {
            let u = app.user_cache.get(id).unwrap_or(&m.user);
            return app.shown_name_for_user(Some(gid.as_str()), u);
        }
    }
    format!("user-{}", &id[id.len().saturating_sub(4)..])
}

// ----- bold, italic and the rest

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Strong,
    Emphasis,
    StrongEmphasis,
    Underline,
    Strikethrough,
    Spoiler,
    Code,
}

#[derive(Clone, Copy, Debug)]
struct Marker {
    kind: Kind,
    len: usize,
    first: u8,
}

fn marker_at(text: &str) -> Option<Marker> {
    let f = byte_at(text, 0);
    let s = byte_at(text, 1);
    let t = byte_at(text, 2);
    let m = |kind, len| {
        Some(Marker {
            kind,
            len,
            first: f,
        })
    };
    if (f == b'*' || f == b'_') && s == f && t == f {
        return m(Kind::StrongEmphasis, 3);
    }
    if f == b'|' && s == b'|' {
        return m(Kind::Spoiler, 2);
    }
    if f == b'~' && s == b'~' {
        return m(Kind::Strikethrough, 2);
    }
    if f == b'*' && s == b'*' {
        return m(Kind::Strong, 2);
    }
    if f == b'_' && s == b'_' {
        return m(Kind::Underline, 2);
    }
    if f == b'`' {
        let run = text.bytes().take_while(|b| *b == b'`').count();
        return m(Kind::Code, run);
    }
    if f == b'*' || f == b'_' {
        return m(Kind::Emphasis, 1);
    }
    None
}

fn is_left_flanking(previous: u8, next: u8) -> bool {
    if is_ws(next) || next == 0 {
        return false;
    }
    !is_punct(next) || is_ws(previous) || previous == 0 || is_punct(previous)
}

fn is_right_flanking(previous: u8, next: u8) -> bool {
    if is_ws(previous) || previous == 0 {
        return false;
    }
    !is_punct(previous) || is_ws(next) || next == 0 || is_punct(next)
}

fn can_open_single(marker: u8, previous: u8, next: u8) -> bool {
    is_left_flanking(previous, next)
        && (marker != b'_' || !is_right_flanking(previous, next) || is_punct(previous))
}

fn can_close_single(marker: u8, previous: u8, next: u8) -> bool {
    is_right_flanking(previous, next)
        && (marker != b'_' || !is_left_flanking(previous, next) || is_punct(next))
}

/// Where the closing marker of `marker` starts in `text` (which begins
/// with the opening one), if it closes at all.
fn formatting_end(text: &str, marker: Marker) -> Option<usize> {
    let mut pos = marker.len;
    if text.len() < marker.len * 2 {
        return None;
    }
    if marker.kind == Kind::Code && marker.len > 1 {
        while pos < text.len() {
            let c = byte_at(text, pos);
            if c == b'\n' || c == b'\r' {
                return None;
            }
            if c == b'`' {
                let count = text[pos..].bytes().take_while(|b| *b == b'`').count();
                if count >= marker.len {
                    return Some(pos + count - marker.len);
                }
                pos += count;
                continue;
            }
            pos += advance_one(text, pos);
        }
        return None;
    }
    if marker.kind == Kind::Code {
        while pos < text.len() {
            let c = byte_at(text, pos);
            if c == b'\n' || c == b'\r' {
                return None;
            }
            if c == b'\\' && pos + 1 < text.len() {
                pos += 2;
                continue;
            }
            if c == b'`' {
                if byte_at(text, pos + 1) == b'`' {
                    pos += text[pos..].bytes().take_while(|b| *b == b'`').count();
                    continue;
                }
                return Some(pos);
            }
            pos += advance_one(text, pos);
        }
        return None;
    }
    if marker.kind == Kind::Emphasis {
        while pos < text.len() {
            let c = byte_at(text, pos);
            if c == b'\\' && pos + 1 < text.len() {
                pos += 2;
                continue;
            }
            if c == marker.first {
                if marker.first == b'*'
                    && let Some(after) = nested_asterisk_run_end(text, pos)
                {
                    pos = after;
                    continue;
                }
                if marker.first == b'_' && byte_at(text, pos + 1) == b'_' {
                    pos += 2;
                    continue;
                }
                let prev = if pos > 0 { byte_at(text, pos - 1) } else { 0 };
                let next = byte_at(text, pos + 1);
                if can_close_single(marker.first, prev, next) {
                    return Some(pos);
                }
            }
            pos += advance_one(text, pos);
        }
        return None;
    }
    let marker_text: String = std::iter::repeat_n(marker.first as char, marker.len).collect();
    let double = marker.len > 1;
    let mut nested = 0usize;
    while pos < text.len() {
        if byte_at(text, pos) == b'\\' && pos + 1 < text.len() {
            pos += 2;
            continue;
        }
        if text[pos..].starts_with(marker_text.as_str()) {
            if nested == 0 {
                if marker.kind == Kind::Spoiler
                    && pos == marker.len
                    && pos + marker.len < text.len()
                {
                    pos += 1;
                    continue;
                }
                return Some(pos);
            }
            nested -= 1;
            pos += marker.len;
            continue;
        }
        if double && byte_at(text, pos) == marker.first && byte_at(text, pos + 1) == marker.first {
            nested += 1;
        }
        pos += advance_one(text, pos);
    }
    None
}

/// Inside `*italic*`, a run of two or more asterisks that opens a bold
/// span is skipped as a whole: the position after its closing marker.
fn nested_asterisk_run_end(text: &str, pos: usize) -> Option<usize> {
    let run = text[pos..].bytes().take_while(|b| *b == b'*').count();
    if run < 2 {
        return None;
    }
    let previous = if pos > 0 { byte_at(text, pos - 1) } else { 0 };
    let next = byte_at(text, pos + run);
    if !is_left_flanking(previous, next) {
        return None;
    }
    let nested = marker_at(&text[pos..])?;
    if nested.first != b'*' || nested.len < 2 {
        return None;
    }
    let inner_end = formatting_end(&text[pos..], nested)?;
    let end = pos + inner_end + nested.len;
    (end > pos && end <= text.len()).then_some(end)
}

fn unescape_inline_code(content: &str) -> String {
    if !content.contains("\\`") {
        return content.to_string();
    }
    let mut out = String::new();
    let bytes = content.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            let n = advance_one(content, i);
            out.push_str(&content[i..i + n]);
            i += n;
            continue;
        }
        let count = bytes[i..].iter().take_while(|b| **b == b'\\').count();
        if byte_at(content, i + count) == b'`' {
            out.push_str(&"\\".repeat(count / 2));
            out.push('`');
            i += count + 1;
        } else {
            out.push_str(&content[i..i + count]);
            i += count;
        }
    }
    out
}

/// A formatting span at the start of `text`: its spans go to `out` and
/// its length comes back.
fn formatting(
    text: &str,
    previous: u8,
    app: &App,
    base: Style,
    ctx: &Ctx,
    out: &mut Vec<Span<'static>>,
    buf: &mut String,
) -> Option<usize> {
    if text.len() < 2 {
        return None;
    }
    let marker = marker_at(text)?;
    if !ctx.can_enter(marker.first, marker.len > 1) {
        return None;
    }
    if marker.kind == Kind::Emphasis && !can_open_single(marker.first, previous, byte_at(text, 1)) {
        return None;
    }
    let end = formatting_end(text, marker)?;
    let inner = &text[marker.len..end];
    if marker.kind == Kind::Code {
        let code = unescape_inline_code(inner);
        if !has_visible_content(&code) {
            return None;
        }
        flush(buf, base, out);
        out.push(Span::styled(
            code,
            base.patch(crate::ui::theme::code_style()),
        ));
        return Some(end + marker.len);
    }
    if !has_visible_content(inner) {
        return None;
    }
    let style = match marker.kind {
        Kind::Strong => base.add_modifier(Modifier::BOLD),
        Kind::Emphasis => base.add_modifier(Modifier::ITALIC),
        Kind::StrongEmphasis => base.add_modifier(Modifier::BOLD | Modifier::ITALIC),
        Kind::Underline => base.add_modifier(Modifier::UNDERLINED),
        Kind::Strikethrough => base.add_modifier(Modifier::CROSSED_OUT),
        Kind::Spoiler => base.patch(spoiler_style()),
        Kind::Code => unreachable!(),
    };
    let inner_ctx = if marker.kind == Kind::StrongEmphasis {
        ctx.with(b'*', true)
    } else {
        ctx.with(marker.first, marker.len > 1)
    };
    flush(buf, base, out);
    parse_inline(inner, app, style, &inner_ctx, out);
    Some(end + marker.len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::WellKnownFluxerResponse;
    use crate::app::ServerSelection;
    use crate::config::UiSettings;

    fn test_app() -> App {
        App::new(
            WellKnownFluxerResponse::default(),
            crate::api::types::UserPrivateResponse {
                id: "me".to_string(),
                ..crate::api::types::UserPrivateResponse::default()
            },
            None,
            Vec::new(),
            Vec::new(),
            ServerSelection::DirectMessages,
            None,
            UiSettings::default(),
        )
    }

    fn joined(spans: &[Span<'static>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    /// The rows of a message as plain text.
    fn rows(content: &str) -> Vec<String> {
        let app = test_app();
        content_lines(content, &app)
            .iter()
            .map(|r| joined(r))
            .collect()
    }

    fn spans(text: &str) -> Vec<Span<'static>> {
        parse_message_spans(text, &test_app())
    }

    fn span_with<'a>(spans: &'a [Span<'static>], content: &str) -> &'a Span<'static> {
        spans
            .iter()
            .find(|s| s.content == content)
            .unwrap_or_else(|| panic!("no span {content:?} in {:?}", joined(spans)))
    }

    fn has(spans: &[Span<'static>], content: &str, m: Modifier) -> bool {
        span_with(spans, content).style.add_modifier.contains(m)
    }

    // ----- inline

    #[test]
    fn the_basic_markers_format_and_disappear() {
        let s = spans("**bold** _under_ __line__ ~~gone~~ ||psst|| `code` *it*");
        assert_eq!(joined(&s), "bold under line gone psst code it");
        assert!(has(&s, "bold", Modifier::BOLD));
        assert!(has(&s, "under", Modifier::ITALIC));
        assert!(has(&s, "line", Modifier::UNDERLINED));
        assert!(has(&s, "gone", Modifier::CROSSED_OUT));
        assert!(has(&s, "psst", Modifier::DIM));
        assert!(has(&s, "it", Modifier::ITALIC));
        assert_eq!(span_with(&s, "code").style, crate::ui::theme::code_style());
    }

    #[test]
    fn triple_markers_and_nesting() {
        let s = spans("***both*** and **bold *and italic* inside**");
        assert_eq!(joined(&s), "both and bold and italic inside");
        assert!(has(&s, "both", Modifier::BOLD | Modifier::ITALIC));
        assert!(has(&s, "bold ", Modifier::BOLD));
        assert!(!has(&s, "bold ", Modifier::ITALIC));
        assert!(has(&s, "and italic", Modifier::BOLD | Modifier::ITALIC));
    }

    #[test]
    fn underscores_inside_words_and_lonely_markers_stay_text() {
        assert_eq!(
            joined(&spans("snake_case_name stays")),
            "snake_case_name stays"
        );
        assert_eq!(joined(&spans("**unclosed bold")), "**unclosed bold");
        assert_eq!(joined(&spans("2 * 3 * 4 = 24")), "2 * 3 * 4 = 24");
        let s = spans("2 * 3 and *x*");
        assert_eq!(joined(&s), "2 * 3 and x");
        assert!(has(&s, "x", Modifier::ITALIC));
    }

    #[test]
    fn backslashes_escape_and_inline_code_keeps_its_backticks() {
        assert_eq!(
            joined(&spans("\\*not italic\\* \\_nor\\_ 100\\% \\\\")),
            "*not italic* _nor_ 100\\% \\"
        );
        let s = spans("``a `b` c`` and ` `");
        assert_eq!(joined(&s), "a `b` c and ` `");
        assert_eq!(
            span_with(&s, "a `b` c").style,
            crate::ui::theme::code_style()
        );
    }

    #[test]
    fn bare_urls_are_links_without_their_trailing_punctuation() {
        let s = spans("see https://example.org/a_b_c. ok (https://x.io/p) and https://a.co.uk");
        assert_eq!(
            joined(&s),
            "see https://example.org/a_b_c. ok (https://x.io/p) and https://a.co.uk"
        );
        let link = Style::default().fg(crate::ui::theme::link_color());
        assert_eq!(span_with(&s, "https://example.org/a_b_c").style, link);
        assert_eq!(span_with(&s, "https://x.io/p").style, link);
        assert_eq!(span_with(&s, "https://a.co.uk").style, link);
        assert!(!has(&s, "https://example.org/a_b_c", Modifier::UNDERLINED));
    }

    #[test]
    fn masked_links_show_the_label_and_the_host_but_never_lie() {
        let s = spans("[Fluxer](https://fluxer.app/x) and <https://fluxer.app>");
        assert_eq!(joined(&s), "Fluxer (fluxer.app) and https://fluxer.app");
        assert!(has(&s, "Fluxer", Modifier::UNDERLINED));
        assert_eq!(
            span_with(&s, "https://fluxer.app").style.fg,
            Some(crate::ui::theme::link_color())
        );
        // a label that is a different URL is shown as written
        assert_eq!(
            joined(&spans("[https://a.com](https://b.com)")),
            "[https://a.com](https://b.com)"
        );
        // the same URL as label needs no host
        assert_eq!(
            joined(&spans("[https://a.com/](https://a.com)")),
            "https://a.com/"
        );
        assert_eq!(joined(&spans("[note](not a url)")), "[note](not a url)");
        assert_eq!(joined(&spans("[x] (y)")), "[x] (y)");
    }

    #[test]
    fn mentions_everyone_and_commands() {
        let s = spans("<@42> <@!42> <#5> </play now:12> @everyone @here <id:browse>");
        assert_eq!(
            joined(&s),
            "@user-42 @user-42 #unknown-5 /play now @everyone @here Browse Channels"
        );
        assert!(has(&s, "@everyone", Modifier::BOLD));
        assert!(has(&s, "/play now", Modifier::BOLD));
        assert_eq!(
            joined(&spans("<@notdigits> <#> <t:> </:1>")),
            "<@notdigits> <#> <t:> </:1>"
        );
    }

    #[test]
    fn timestamps_in_every_style_and_relative() {
        // noon UTC, so the date is the same in every time zone that is
        // less than twelve hours away from UTC
        let s = spans("<t:1700049600:d> <t:1700049600:D>");
        assert_eq!(joined(&s), "2023-11-15 November 15, 2023");
        assert!(!joined(&spans("<t:1700049600>")).contains("<t:"));
        assert!(!joined(&spans("<t:1700049600:F>")).contains("<t:"));
        assert!(joined(&spans("<t:1700049600:R>")).ends_with(" ago"));
        assert_eq!(joined(&spans("<t:1700049600:x>")), "<t:1700049600:x>");
        let now = 1_800_000_000;
        assert_eq!(relative_time(now - 3 * 3600, now), "3 hours ago");
        assert_eq!(relative_time(now - 3600, now), "an hour ago");
        assert_eq!(relative_time(now + 90 * 86400, now), "in 3 months");
        assert_eq!(relative_time(now + 10, now), "in a moment");
        assert_eq!(relative_time(now - 2 * 366 * 86400, now), "2 years ago");
    }

    #[test]
    fn emoji_by_shortcode_custom_and_literal() {
        let s = spans(":thumbsup: <:pepe:123> <a:wave:456> 😀 x");
        assert_eq!(joined(&s), "👍 :pepe: :wave: 😀 x");
        let emoji = crate::ui::theme::emoji_unknown();
        assert_eq!(span_with(&s, "👍").style.fg, Some(emoji));
        assert_eq!(span_with(&s, "😀").style.fg, Some(emoji));
        assert_eq!(span_with(&s, ":pepe:").style.fg, Some(emoji));
        assert!(has(&s, ":wave:", Modifier::ITALIC));
        assert_eq!(joined(&spans("\\:thumbsup: stays")), ":thumbsup: stays");
    }

    // ----- blocks

    #[test]
    fn headings_and_subtext() {
        assert_eq!(rows("# Title\ntext"), ["# Title", "text"]);
        assert_eq!(rows("#### Four\n##### five"), ["#### Four", "##### five"]);
        assert_eq!(rows("#Nope"), ["#Nope"]);
        let app = test_app();
        let out = content_lines("-# small print\n-#  two spaces", &app);
        assert_eq!(joined(&out[0]), "small print");
        assert_eq!(out[0][0].style, crate::ui::theme::muted_style());
        assert_eq!(joined(&out[1]), "-#  two spaces");
    }

    #[test]
    fn quotes_single_and_for_the_rest_of_the_message() {
        assert_eq!(rows("> a\n> b\nc"), ["\u{258E} a", "\u{258E} b", "c"]);
        // a quote's leading blank line is dropped, like the web app does
        assert_eq!(rows("> \n> x"), ["\u{258E} x"]);
        assert_eq!(
            rows("> x\n> \n> y"),
            ["\u{258E} x", "\u{258E} ", "\u{258E} y"]
        );
        assert_eq!(
            rows(">>> a\nb\n> c"),
            ["\u{258E} a", "\u{258E} b", "\u{258E} \u{258E} c"]
        );
        assert_eq!(rows(">not a quote"), [">not a quote"]);
    }

    #[test]
    fn alerts_get_a_badge_and_a_coloured_bar() {
        assert_eq!(
            rows("> [!WARNING]\n> careful\n> now"),
            ["\u{258E}  WARNING ", "\u{258E} careful", "\u{258E} now"]
        );
        assert_eq!(
            rows("> [!tip] one-liner"),
            ["\u{258E}  TIP ", "\u{258E} one-liner"]
        );
        assert_eq!(rows("> [!SHOUT] x"), ["\u{258E} [!SHOUT] x"]);
        let app = test_app();
        let out = content_lines("> [!CAUTION]\n> boom", &app);
        assert_eq!(out[0][0].style.fg, Some(crate::ui::theme::danger()));
    }

    #[test]
    fn lists_bullets_numbers_nesting_and_continuation() {
        assert_eq!(
            rows("- a\n- b\n  - c\n    - d\n* e"),
            [
                "\u{2022} a",
                "\u{2022} b",
                "  \u{25E6} c",
                "    \u{25AA} d",
                "\u{2022} e"
            ]
        );
        assert_eq!(rows("3. x\n4. y\n7. z"), ["3. x", "4. y", "5. z"]);
        assert_eq!(
            rows("- a\n   more of a\n- b"),
            ["\u{2022} a", "  more of a", "\u{2022} b"]
        );
        assert_eq!(rows("-nope\n -one space"), ["-nope", " -one space"]);
        assert_eq!(rows("1. one\n- two"), ["1. one", "\u{2022} two"]);
    }

    #[test]
    fn code_blocks_with_and_without_a_language() {
        assert_eq!(
            rows("```rust\nfn main() {}\n```\nafter"),
            ["\u{2503} rust", "\u{2503} fn main() {}", "after"]
        );
        assert_eq!(
            rows("```\nno lang\n\tindented\n```"),
            ["\u{2503} no lang", "\u{2503}     indented"]
        );
        assert_eq!(
            rows("```not a lang\nx\n```"),
            ["\u{2503} not a lang", "\u{2503} x"]
        );
        assert_eq!(rows("```unclosed\nx"), ["```unclosed", "x"]);
        assert_eq!(rows("```one liner```"), ["\u{2503} one liner"]);
        assert_eq!(rows("text ```code```"), ["text", "\u{2503} code"]);
        assert_eq!(rows("```\nx\n``` tail"), ["\u{2503} x", "tail"]);
        assert_eq!(rows("```\n\n```"), ["```", "", "```"]);
        let app = test_app();
        let out = content_lines("```\nlet x = **not bold**;\n```", &app);
        assert_eq!(joined(&out[0]), "\u{2503} let x = **not bold**;");
        assert_eq!(out[0][1].style, crate::ui::theme::code_style());
    }

    #[test]
    fn spoilers_over_several_lines() {
        let app = test_app();
        let out = content_lines("||secret\nlines|| tail", &app);
        assert_eq!(joined(&out[0]), "secret");
        assert_eq!(joined(&out[1]), "lines");
        assert_eq!(joined(&out[2]), "tail");
        assert!(out[0][0].style.add_modifier.contains(Modifier::DIM));
        assert!(!out[2][0].style.add_modifier.contains(Modifier::DIM));
        assert_eq!(rows("||never closed\nx"), ["||never closed", "x"]);
    }

    #[test]
    fn tables_align_their_columns() {
        assert_eq!(
            rows("| a | bb |\n|---|:-:|\n| 111 | 2 |\nafter"),
            [
                "a   \u{2502} bb",
                "\u{2500}\u{2500}\u{2500}\u{2500}\u{253C}\u{2500}\u{2500}\u{2500}",
                "111 \u{2502} 2 ",
                "after"
            ]
        );
        assert_eq!(
            rows("| r |\n|--:|\n| 1 |\n| 22 |"),
            [" r", "\u{2500}\u{2500}", " 1", "22"]
        );
        // a header without a body is not a table
        assert_eq!(rows("| a |\n|---|"), ["| a |", "|---|"]);
    }

    #[test]
    fn blank_lines_and_inline_markup_across_lines() {
        assert_eq!(rows("a\n\nb"), ["a", "", "b"]);
        assert_eq!(rows("\na\n\n"), ["a"]);
        assert_eq!(rows("a\n\n\n# h\n\nb"), ["a", "# h", "b"]);
        let app = test_app();
        let out = content_lines("**bold\nstill** plain", &app);
        assert_eq!(joined(&out[0]), "bold");
        assert_eq!(joined(&out[1]), "still plain");
        assert!(out[1][0].style.add_modifier.contains(Modifier::BOLD));
        assert!(!out[1][1].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(rows("text\n# head\n> q"), ["text", "# head", "\u{258E} q"]);
    }

    #[test]
    fn helpers() {
        assert!(has_open_inline_code("a `b"));
        assert!(!has_open_inline_code("a `b` c"));
        assert!(has_open_inline_code("``a` b"));
        assert_eq!(
            url_segment("https://a.com/x?y=1)."),
            Some("https://a.com/x?y=1")
        );
        assert_eq!(url_segment("https://a.com/(x)"), Some("https://a.com/(x)"));
        assert_eq!(url_segment("https://"), None);
        assert!(is_code_fence_language("rust"));
        assert!(is_code_fence_language("c++"));
        assert!(!is_code_fence_language("not a lang"));
        assert!(!is_code_fence_language(" rust"));
    }
}
