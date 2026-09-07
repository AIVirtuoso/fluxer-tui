//! Text operations for the compose box.
//!
//! The text being composed lives in two strings: `head` is everything
//! before the cursor and `tail` everything after it. Typing pushes onto
//! the head, so the autocomplete code that looks at the end of the input
//! keeps working unchanged; moving the cursor shifts text between the two.
//! A custom emoji token (`<:name:id>`) is one unit for every movement and
//! deletion, as it is one glyph on screen.
//!
//! Everything here is pure string work so it can be tested without an
//! `App`. Positions are byte offsets into the full text `head + tail`.

use crate::app::parse_custom_emoji_token;

/// The full text, cursor at `head.len()`.
pub fn full(head: &str, tail: &str) -> String {
    let mut s = String::with_capacity(head.len() + tail.len());
    s.push_str(head);
    s.push_str(tail);
    s
}

/// Byte length of the unit (custom emoji token or one char) that ends the head.
fn unit_len_before(head: &str) -> usize {
    if head.ends_with('>')
        && let Some(lt) = head.rfind('<')
        && let Some(tok) = parse_custom_emoji_token(&head[lt..])
        && lt + tok.len == head.len()
    {
        return tok.len;
    }
    head.chars().next_back().map(char::len_utf8).unwrap_or(0)
}

/// Byte length of the unit (custom emoji token or one char) that starts the tail.
fn unit_len_after(tail: &str) -> usize {
    if tail.starts_with('<')
        && let Some(tok) = parse_custom_emoji_token(tail)
    {
        return tok.len;
    }
    tail.chars().next().map(char::len_utf8).unwrap_or(0)
}

pub fn move_left(head: &mut String, tail: &mut String) {
    let n = unit_len_before(head);
    if n == 0 {
        return;
    }
    let at = head.len() - n;
    tail.insert_str(0, &head[at..]);
    head.truncate(at);
}

pub fn move_right(head: &mut String, tail: &mut String) {
    let n = unit_len_after(tail);
    if n == 0 {
        return;
    }
    head.push_str(&tail[..n]);
    tail.drain(..n);
}

/// Put the cursor at byte offset `pos` of the full text (clamped to a
/// char boundary at or before `pos`).
pub fn set_cursor(head: &mut String, tail: &mut String, pos: usize) {
    let text = full(head, tail);
    let mut pos = pos.min(text.len());
    while !text.is_char_boundary(pos) {
        pos -= 1;
    }
    head.clear();
    head.push_str(&text[..pos]);
    tail.clear();
    tail.push_str(&text[pos..]);
}

/// Byte offset where the cursor's line starts in the full text.
pub fn line_start(head: &str) -> usize {
    head.rfind('\n').map(|i| i + 1).unwrap_or(0)
}

/// Byte offset where the cursor's line ends in the full text.
pub fn line_end(head: &str, tail: &str) -> usize {
    head.len() + tail.find('\n').unwrap_or(tail.len())
}

pub fn move_home(head: &mut String, tail: &mut String) {
    let at = line_start(head);
    set_cursor(head, tail, at);
}

pub fn move_end(head: &mut String, tail: &mut String) {
    let at = line_end(head, tail);
    set_cursor(head, tail, at);
}

/// Start of the whitespace-separated word before the cursor (readline's
/// backward-word, the same chunking as Ctrl+Backspace).
pub fn word_start_before(head: &str) -> usize {
    let mut i = head.len();
    let bytes = head.as_bytes();
    while i > 0 && (bytes[i - 1] as char).is_ascii_whitespace() {
        i -= 1;
    }
    while i > 0 && !(bytes[i - 1] as char).is_ascii_whitespace() {
        i -= 1;
    }
    i
}

/// End of the whitespace-separated word after the cursor.
pub fn word_end_after(tail: &str) -> usize {
    let bytes = tail.as_bytes();
    let mut i = 0;
    while i < bytes.len() && (bytes[i] as char).is_ascii_whitespace() {
        i += 1;
    }
    while i < bytes.len() && !(bytes[i] as char).is_ascii_whitespace() {
        i += 1;
    }
    i
}

pub fn move_word_left(head: &mut String, tail: &mut String) {
    let at = word_start_before(head);
    set_cursor(head, tail, at);
}

pub fn move_word_right(head: &mut String, tail: &mut String) {
    let at = head.len() + word_end_after(tail);
    set_cursor(head, tail, at);
}

/// Column of the cursor on its line, in chars.
fn cursor_col(head: &str) -> usize {
    head[line_start(head)..].chars().count()
}

/// Byte offset of the char at column `col` of the line starting at `start`
/// in `text`, or the line's end when it is shorter.
fn offset_at_col(text: &str, start: usize, col: usize) -> usize {
    let line = &text[start..];
    let line = &line[..line.find('\n').unwrap_or(line.len())];
    line.char_indices()
        .nth(col)
        .map(|(i, _)| start + i)
        .unwrap_or(start + line.len())
}

/// Move to the same column on the previous line. False on the first line,
/// where the caller decides what Up means.
pub fn move_line_up(head: &mut String, tail: &mut String) -> bool {
    let cur_start = line_start(head);
    if cur_start == 0 {
        return false;
    }
    let col = cursor_col(head);
    let prev_start = head[..cur_start - 1]
        .rfind('\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    let text = full(head, tail);
    let at = offset_at_col(&text, prev_start, col);
    set_cursor(head, tail, at);
    true
}

/// Move to the same column on the next line. False on the last line.
pub fn move_line_down(head: &mut String, tail: &mut String) -> bool {
    let Some(nl) = tail.find('\n') else {
        return false;
    };
    let col = cursor_col(head);
    let next_start = head.len() + nl + 1;
    let text = full(head, tail);
    let at = offset_at_col(&text, next_start, col);
    set_cursor(head, tail, at);
    true
}

/// Delete the unit after the cursor (the Delete key).
pub fn delete_forward(head: &str, tail: &mut String) {
    let _ = head;
    let n = unit_len_after(tail);
    tail.drain(..n);
}

/// Delete the whitespace-separated word after the cursor.
pub fn delete_word_forward(tail: &mut String) {
    let n = word_end_after(tail);
    tail.drain(..n);
}

/// Delete the whitespace-separated word before the cursor.
pub fn delete_word_backward(head: &mut String) {
    let at = word_start_before(head);
    head.truncate(at);
}

/// Delete from the cursor to the end of its line; on an empty line, the
/// line break itself (readline's kill-line).
pub fn kill_to_line_end(tail: &mut String) {
    match tail.find('\n') {
        Some(0) => {
            tail.drain(..1);
        }
        Some(n) => {
            tail.drain(..n);
        }
        None => tail.clear(),
    }
}

/// Remove the selected range `[start, end)` of the full text; the cursor
/// lands at `start`. Returns the removed text.
pub fn delete_range(head: &mut String, tail: &mut String, start: usize, end: usize) -> String {
    let text = full(head, tail);
    let (start, end) = clamp_range(&text, start, end);
    let removed = text[start..end].to_string();
    head.clear();
    head.push_str(&text[..start]);
    tail.clear();
    tail.push_str(&text[end..]);
    removed
}

/// Clamp a byte range to char boundaries inside `text`.
pub fn clamp_range(text: &str, start: usize, end: usize) -> (usize, usize) {
    let mut s = start.min(text.len());
    let mut e = end.min(text.len());
    while !text.is_char_boundary(s) {
        s -= 1;
    }
    while !text.is_char_boundary(e) {
        e -= 1;
    }
    if s > e { (e, s) } else { (s, e) }
}

/// The word around the cursor when the cursor touches one: byte range in
/// the full text. Whitespace-separated, like the word movements.
pub fn word_at_cursor(head: &str, tail: &str) -> Option<(usize, usize)> {
    let before = head
        .chars()
        .next_back()
        .map(|c| !c.is_whitespace())
        .unwrap_or(false);
    let after = tail
        .chars()
        .next()
        .map(|c| !c.is_whitespace())
        .unwrap_or(false);
    if !before && !after {
        return None;
    }
    let start = if before {
        word_start_before(head)
    } else {
        head.len()
    };
    let end = if after {
        head.len() + word_end_after(tail)
    } else {
        head.len()
    };
    Some((start, end))
}

/// What a formatting shortcut did, so the caller can restore a selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WrapResult {
    /// Range of the text between the markers, in the new full text.
    pub start: usize,
    pub end: usize,
}

/// Whether a run of `run` marker characters carries this marker. The
/// markers are runs of one character, and `*` is ambiguous: `**x**` is
/// bold only, `***x***` is bold and italic, so italic is present when
/// the run is odd; every other marker is present when the run is long
/// enough.
fn marker_present(run: usize, marker: &str) -> bool {
    let m = marker.len();
    if run < m {
        return false;
    }
    if marker == "*" { run % 2 == 1 } else { true }
}

/// Toggle markdown markers around `[start, end)` like the web composer:
/// already wrapped (inside the range or just around it) unwraps, otherwise
/// the range is wrapped. The cursor ends at the end of the text between
/// the markers, so the selection can be restored to it.
pub fn toggle_wrap(
    head: &mut String,
    tail: &mut String,
    start: usize,
    end: usize,
    marker: &str,
) -> WrapResult {
    let text = full(head, tail);
    let (start, end) = clamp_range(&text, start, end);
    let selected = &text[start..end];
    let m = marker.len();
    let c = marker.chars().next().unwrap_or('*');
    let run_start = selected.chars().take_while(|&x| x == c).count();
    let run_end = selected.chars().rev().take_while(|&x| x == c).count();
    let wrapped_inside = selected.len() >= 2 * m && marker_present(run_start.min(run_end), marker);
    let run_before = text[..start].chars().rev().take_while(|&x| x == c).count();
    let run_after = text[end..].chars().take_while(|&x| x == c).count();
    let wrapped_around = !wrapped_inside && marker_present(run_before.min(run_after), marker);
    let (new_text, r) = if wrapped_inside {
        let inner = &selected[m..selected.len() - m];
        (
            format!("{}{}{}", &text[..start], inner, &text[end..]),
            WrapResult {
                start,
                end: start + inner.len(),
            },
        )
    } else if wrapped_around {
        (
            format!("{}{}{}", &text[..start - m], selected, &text[end + m..]),
            WrapResult {
                start: start - m,
                end: end - m,
            },
        )
    } else {
        (
            format!(
                "{}{marker}{selected}{marker}{}",
                &text[..start],
                &text[end..]
            ),
            WrapResult {
                start: start + m,
                end: end + m,
            },
        )
    };
    head.clear();
    head.push_str(&new_text[..r.end]);
    tail.clear();
    tail.push_str(&new_text[r.end..]);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ht(h: &str, t: &str) -> (String, String) {
        (h.to_string(), t.to_string())
    }

    #[test]
    fn left_right_move_chars_and_emoji_tokens_as_units() {
        let (mut h, mut t) = ht("ab<:wave:123>", "");
        move_left(&mut h, &mut t);
        assert_eq!((h.as_str(), t.as_str()), ("ab", "<:wave:123>"));
        move_left(&mut h, &mut t);
        assert_eq!((h.as_str(), t.as_str()), ("a", "b<:wave:123>"));
        move_right(&mut h, &mut t);
        move_right(&mut h, &mut t);
        assert_eq!((h.as_str(), t.as_str()), ("ab<:wave:123>", ""));
        move_right(&mut h, &mut t);
        assert_eq!(h, "ab<:wave:123>");
        let (mut h, mut t) = ht("é", "ü");
        move_left(&mut h, &mut t);
        assert_eq!((h.as_str(), t.as_str()), ("", "éü"));
    }

    #[test]
    fn home_end_stay_on_the_cursor_line() {
        let (mut h, mut t) = ht("first\nsec", "ond\nthird");
        move_home(&mut h, &mut t);
        assert_eq!((h.as_str(), t.as_str()), ("first\n", "second\nthird"));
        move_end(&mut h, &mut t);
        assert_eq!((h.as_str(), t.as_str()), ("first\nsecond", "\nthird"));
    }

    #[test]
    fn word_moves_and_deletes_use_whitespace_chunks() {
        let (mut h, mut t) = ht("one two  ", "three four");
        move_word_left(&mut h, &mut t);
        assert_eq!(h, "one ");
        move_word_right(&mut h, &mut t);
        assert_eq!((h.as_str(), t.as_str()), ("one two", "  three four"));
        move_word_right(&mut h, &mut t);
        assert_eq!(h, "one two  three");
        delete_word_forward(&mut t);
        assert_eq!(t, "");
        delete_word_backward(&mut h);
        assert_eq!(h, "one two  ");
    }

    #[test]
    fn line_up_and_down_keep_the_column_when_they_can() {
        let (mut h, mut t) = ht("abcdef\nxy", "z\nlong line here");
        assert!(move_line_up(&mut h, &mut t));
        assert_eq!(h, "ab");
        assert!(!move_line_up(&mut h, &mut t));
        assert!(move_line_down(&mut h, &mut t));
        assert_eq!(h, "abcdef\nxy");
        assert!(move_line_down(&mut h, &mut t));
        assert_eq!(h, "abcdef\nxyz\nlo");
        assert!(!move_line_down(&mut h, &mut t));
        // a shorter target line clamps to its end
        let (mut h, mut t) = ht("abcdef\nxy\nabcd", "ef");
        assert!(move_line_up(&mut h, &mut t));
        assert_eq!((h.as_str(), t.as_str()), ("abcdef\nxy", "\nabcdef"));
    }

    #[test]
    fn delete_forward_and_kill_line() {
        let (h, mut t) = ht("a", "<:x:1>bc\nnext");
        delete_forward(&h, &mut t);
        assert_eq!(t, "bc\nnext");
        kill_to_line_end(&mut t);
        assert_eq!(t, "\nnext");
        kill_to_line_end(&mut t);
        assert_eq!(t, "next");
        kill_to_line_end(&mut t);
        assert_eq!(t, "");
    }

    #[test]
    fn delete_range_puts_the_cursor_at_the_start() {
        let (mut h, mut t) = ht("hello wor", "ld!");
        let gone = delete_range(&mut h, &mut t, 6, 11);
        assert_eq!(gone, "world");
        assert_eq!((h.as_str(), t.as_str()), ("hello ", "!"));
    }

    #[test]
    fn word_at_cursor_finds_the_touching_word() {
        let (h, t) = ht("say hel", "lo there");
        assert_eq!(word_at_cursor(&h, &t), Some((4, 9)));
        let (h, t) = ht("say ", " there");
        assert_eq!(word_at_cursor(&h, &t), None);
        let (h, t) = ht("say", " there");
        assert_eq!(word_at_cursor(&h, &t), Some((0, 3)));
    }

    #[test]
    fn toggle_wrap_wraps_then_unwraps() {
        let (mut h, mut t) = ht("make this", " bold");
        let r = toggle_wrap(&mut h, &mut t, 5, 9, "**");
        assert_eq!(full(&h, &t), "make **this** bold");
        assert_eq!((r.start, r.end), (7, 11));
        // the cursor stays at the end of the text between the markers
        assert_eq!(h, "make **this");
        // the selection now sits inside the markers: toggling again unwraps
        let r = toggle_wrap(&mut h, &mut t, r.start, r.end, "**");
        assert_eq!(full(&h, &t), "make this bold");
        assert_eq!((r.start, r.end), (5, 9));
        // markers inside the selection unwrap too
        let (mut h, mut t) = ht("~~gone~~", "");
        let r = toggle_wrap(&mut h, &mut t, 0, 8, "~~");
        assert_eq!(full(&h, &t), "gone");
        assert_eq!((r.start, r.end), (0, 4));
        // markers stack: italic on bold adds a star, bold on both takes two
        let (mut h, mut t) = ht("**this**", "");
        toggle_wrap(&mut h, &mut t, 0, 8, "*");
        assert_eq!(full(&h, &t), "***this***");
        toggle_wrap(&mut h, &mut t, 0, 10, "**");
        assert_eq!(full(&h, &t), "*this*");
        toggle_wrap(&mut h, &mut t, 0, 6, "*");
        assert_eq!(full(&h, &t), "this");
        // an empty range gets a pair with the cursor between the markers
        let (mut h, mut t) = ht("x ", "");
        toggle_wrap(&mut h, &mut t, 2, 2, "*");
        assert_eq!((h.as_str(), t.as_str()), ("x *", "*"));
    }
}

/// One undo step: the compose text and cursor before an edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputSnapshot {
    pub head: String,
    pub tail: String,
    pub anchor: Option<usize>,
}

/// Kinds of edit for undo grouping: a run of typed characters is one
/// step, a run of Backspaces is one step, anything else is its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEditKind {
    Typing,
    Erasing,
    Discrete,
}

const UNDO_DEPTH: usize = 200;

/// Where a movement key sends the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    Left,
    Right,
    Home,
    End,
    WordLeft,
    WordRight,
    LineUp,
    LineDown,
    TextStart,
    TextEnd,
}

impl crate::app::App {
    /// The whole compose text.
    pub fn input_text(&self) -> String {
        full(&self.input, &self.input_tail)
    }

    pub fn input_is_empty(&self) -> bool {
        self.input.is_empty() && self.input_tail.is_empty()
    }

    /// Chars in the compose text, for the limit and the counter.
    pub fn input_char_count(&self) -> usize {
        self.input.chars().count() + self.input_tail.chars().count()
    }

    /// Take the compose text out, leaving the box empty.
    pub fn take_input(&mut self) -> String {
        let text = self.input_text();
        self.clear_input();
        self.input_reset_history();
        text
    }

    pub fn clear_input(&mut self) {
        self.input.clear();
        self.input_tail.clear();
        self.input_clear_selection();
    }

    /// Replace the compose text; the cursor goes to the end.
    pub fn set_input(&mut self, text: impl Into<String>) {
        self.input = text.into();
        self.input_tail.clear();
        self.input_clear_selection();
    }

    pub fn input_cursor(&self) -> usize {
        self.input.len()
    }

    /// The selected byte range of the full text, when it is not empty.
    pub fn input_selection(&self) -> Option<(usize, usize)> {
        let a = self.input_anchor?;
        let c = self.input_cursor();
        if a == c {
            return None;
        }
        Some((a.min(c), a.max(c)))
    }

    pub fn input_clear_selection(&mut self) {
        self.input_anchor = None;
        self.input_mark = false;
    }

    /// Ctrl+Space: set the mark here, or drop it when one is set.
    pub fn input_toggle_mark(&mut self) {
        if self.input_mark {
            self.input_clear_selection();
        } else {
            self.input_anchor = Some(self.input_cursor());
            self.input_mark = true;
        }
    }

    /// Move the cursor. With `extend` (Shift held) or an active mark the
    /// selection grows from its anchor; otherwise it is dropped. Returns
    /// false when the move was impossible (Up on the first line, Down on
    /// the last), so the caller can give the key its other meaning.
    pub fn input_move(&mut self, mv: Move, extend: bool) -> bool {
        if extend || self.input_mark {
            if self.input_anchor.is_none() {
                self.input_anchor = Some(self.input_cursor());
            }
        } else {
            self.input_anchor = None;
        }
        let (h, t) = (&mut self.input, &mut self.input_tail);
        match mv {
            Move::Left => move_left(h, t),
            Move::Right => move_right(h, t),
            Move::Home => move_home(h, t),
            Move::End => move_end(h, t),
            Move::WordLeft => move_word_left(h, t),
            Move::WordRight => move_word_right(h, t),
            Move::LineUp => return move_line_up(h, t),
            Move::LineDown => return move_line_down(h, t),
            Move::TextStart => set_cursor(h, t, 0),
            Move::TextEnd => {
                let end = h.len() + t.len();
                set_cursor(h, t, end);
            }
        }
        true
    }

    /// Remove the selection, if any, and return its text.
    pub fn input_delete_selection(&mut self) -> Option<String> {
        let (s, e) = self.input_selection()?;
        let gone = delete_range(&mut self.input, &mut self.input_tail, s, e);
        self.input_clear_selection();
        Some(gone)
    }

    /// Backspace: the selection when there is one, else the unit before
    /// the cursor.
    pub fn input_backspace(&mut self) {
        if self.input_delete_selection().is_some() {
            return;
        }
        self.input_clear_selection();
        self.input_pop();
    }

    /// Delete: the selection when there is one, else the unit after the cursor.
    pub fn input_delete_forward(&mut self) {
        if self.input_delete_selection().is_some() {
            return;
        }
        self.input_clear_selection();
        delete_forward(&self.input, &mut self.input_tail);
    }

    pub fn input_delete_word_backward(&mut self) {
        if self.input_delete_selection().is_some() {
            return;
        }
        self.input_clear_selection();
        delete_word_backward(&mut self.input);
    }

    pub fn input_delete_word_forward(&mut self) {
        if self.input_delete_selection().is_some() {
            return;
        }
        self.input_clear_selection();
        delete_word_forward(&mut self.input_tail);
    }

    /// Ctrl+K: the rest of the line goes to the cut buffer.
    pub fn input_kill_to_line_end(&mut self) {
        if let Some(gone) = self.input_delete_selection() {
            self.cut_buffer = gone;
            return;
        }
        let n = match self.input_tail.find('\n') {
            Some(0) => 1,
            Some(n) => n,
            None => self.input_tail.len(),
        };
        if n == 0 {
            return;
        }
        self.cut_buffer = self.input_tail[..n].to_string();
        kill_to_line_end(&mut self.input_tail);
    }

    /// Type one character: it replaces the selection when there is one.
    pub fn input_type(&mut self, ch: char) {
        self.input_delete_selection();
        self.input_clear_selection();
        self.input.push(ch);
    }

    /// Insert text at the cursor, replacing the selection.
    pub fn input_insert_str(&mut self, text: &str) {
        self.input_delete_selection();
        self.input_clear_selection();
        self.input.push_str(text);
    }

    /// Room left under the message length limit, in chars.
    pub fn input_room(&self, max: usize) -> usize {
        max.saturating_sub(self.input_char_count())
    }

    /// Ctrl+B and friends: toggle markdown markers around the selection,
    /// else the word at the cursor, else insert a pair to type into. The
    /// selection is dropped and the cursor lands after the closing marker
    /// (between the markers of a fresh empty pair), so typing goes on
    /// after the word; a second key on the same word stacks its markers.
    pub fn input_toggle_wrap(&mut self, marker: &str) {
        let (s, e) = match self.input_selection() {
            Some(r) => r,
            None => word_at_cursor(&self.input, &self.input_tail)
                .unwrap_or((self.input_cursor(), self.input_cursor())),
        };
        let before = self.input.len() + self.input_tail.len();
        let r = toggle_wrap(&mut self.input, &mut self.input_tail, s, e, marker);
        self.input_clear_selection();
        let wrapped = self.input.len() + self.input_tail.len() > before;
        if wrapped && r.end > r.start {
            set_cursor(&mut self.input, &mut self.input_tail, r.end + marker.len());
        }
    }

    /// Ctrl+C: the selection to the cut buffer and the system clipboard.
    pub fn input_copy(&mut self) -> bool {
        let Some((s, e)) = self.input_selection() else {
            return false;
        };
        let text = self.input_text();
        self.cut_buffer = text[s..e].to_string();
        copy_to_system_clipboard(&self.cut_buffer);
        self.input_clear_selection();
        true
    }

    /// Ctrl+X with a selection: cut it.
    pub fn input_cut(&mut self) -> bool {
        let Some(gone) = self.input_delete_selection() else {
            return false;
        };
        copy_to_system_clipboard(&gone);
        self.cut_buffer = gone;
        true
    }

    /// Alt+V: insert the cut buffer.
    pub fn input_paste_cut_buffer(&mut self) -> bool {
        if self.cut_buffer.is_empty() {
            return false;
        }
        let text = self.cut_buffer.clone();
        self.input_insert_str(&text);
        true
    }

    fn input_snapshot(&self) -> InputSnapshot {
        InputSnapshot {
            head: self.input.clone(),
            tail: self.input_tail.clone(),
            anchor: self.input_anchor,
        }
    }

    /// Call before an edit. Typing runs and Backspace runs are grouped
    /// into one undo step each; a space or newline starts a new group.
    pub fn input_record(&mut self, kind: InputEditKind) {
        let grouped = kind != InputEditKind::Discrete && self.input_last_edit == Some(kind);
        self.input_last_edit = Some(kind);
        self.input_redo.clear();
        if grouped {
            return;
        }
        let snap = self.input_snapshot();
        if self.input_history.last() == Some(&snap) {
            return;
        }
        self.input_history.push(snap);
        if self.input_history.len() > UNDO_DEPTH {
            self.input_history.remove(0);
        }
    }

    /// After a key that turned out not to change the text, the snapshot
    /// taken for it is dropped again.
    pub fn input_forget_noop_record(&mut self) {
        if self.input_history.last() == Some(&self.input_snapshot()) {
            self.input_history.pop();
            self.input_last_edit = None;
        }
    }

    /// The next typed character starts a new undo group.
    pub fn input_break_undo_group(&mut self) {
        self.input_last_edit = None;
    }

    fn input_restore(&mut self, snap: InputSnapshot) {
        self.input = snap.head;
        self.input_tail = snap.tail;
        self.input_anchor = snap.anchor;
        self.input_mark = false;
    }

    pub fn input_undo(&mut self) -> bool {
        let Some(snap) = self.input_history.pop() else {
            return false;
        };
        let now = self.input_snapshot();
        self.input_redo.push(now);
        self.input_restore(snap);
        self.input_last_edit = None;
        true
    }

    pub fn input_redo(&mut self) -> bool {
        let Some(snap) = self.input_redo.pop() else {
            return false;
        };
        let now = self.input_snapshot();
        self.input_history.push(now);
        self.input_restore(snap);
        self.input_last_edit = None;
        true
    }

    /// Sending or clearing the box also empties the undo stacks.
    pub fn input_reset_history(&mut self) {
        self.input_history.clear();
        self.input_redo.clear();
        self.input_last_edit = None;
    }
}

/// Hand text to the system clipboard through wl-copy (Wayland) or xclip
/// (X11), whichever is on PATH; nowhere else, such as on the console, the
/// cut buffer alone keeps it. Fire and forget: the program's exit status
/// is not interesting enough to wait for.
pub fn copy_to_system_clipboard(text: &str) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let candidates: [(&str, &[&str]); 2] = [
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard", "-i"]),
    ];
    for (cmd, args) in candidates {
        let child = Command::new(cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = child else {
            continue;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        return;
    }
}

#[cfg(test)]
mod app_tests {
    use crate::api::types::UserPrivateResponse;
    use crate::app::App;

    fn app() -> App {
        let me = UserPrivateResponse {
            id: "me".into(),
            ..Default::default()
        };
        App::new(
            Default::default(),
            me,
            None,
            Vec::new(),
            Vec::new(),
            crate::app::ServerSelection::DirectMessages,
            None,
            Default::default(),
        )
    }

    #[test]
    fn format_key_wraps_the_word_and_moves_on() {
        let mut a = app();
        a.set_input("make this bold");
        // select "this" with the mark
        a.input_move(super::Move::WordLeft, false);
        a.input_move(super::Move::WordLeft, false);
        a.input_toggle_mark();
        a.input_move(super::Move::WordRight, false);
        assert_eq!(a.input_selection(), Some((5, 9)));
        a.input_toggle_wrap("**");
        assert_eq!(a.input_text(), "make **this** bold");
        assert_eq!(a.input, "make **this**");
        assert!(a.input_selection().is_none());
        // typing goes on after the word, nothing is replaced
        a.input_type('!');
        assert_eq!(a.input_text(), "make **this**! bold");
        // the same key on the word again removes the markers
        a.input_backspace();
        a.input_toggle_wrap("**");
        assert_eq!(a.input_text(), "make this bold");
        // a second marker stacks
        a.input_toggle_wrap("**");
        a.input_toggle_wrap("*");
        assert_eq!(a.input_text(), "make ***this*** bold");
        // nothing at the cursor: an empty pair to type into
        a.set_input("x ");
        a.input_toggle_wrap("`");
        a.input_type('y');
        assert_eq!(a.input_text(), "x `y`");
    }

    #[test]
    fn undo_groups_typing_and_erasing() {
        use super::InputEditKind as K;
        let mut a = app();
        for c in "hello".chars() {
            a.input_record(K::Typing);
            a.input_type(c);
        }
        a.input_record(K::Discrete);
        a.input_type(' ');
        for c in "world".chars() {
            a.input_record(K::Typing);
            a.input_type(c);
        }
        assert_eq!(a.input_text(), "hello world");
        assert!(a.input_undo());
        assert_eq!(a.input_text(), "hello ");
        assert!(a.input_undo());
        assert_eq!(a.input_text(), "hello");
        assert!(a.input_undo());
        assert_eq!(a.input_text(), "");
        assert!(!a.input_undo());
        assert!(a.input_redo());
        assert_eq!(a.input_text(), "hello");
        // a run of Backspaces is one step
        a.input_record(K::Erasing);
        a.input_backspace();
        a.input_record(K::Erasing);
        a.input_backspace();
        assert_eq!(a.input_text(), "hel");
        assert!(a.input_undo());
        assert_eq!(a.input_text(), "hello");
        // a key that changed nothing leaves no step behind
        a.input_record(K::Discrete);
        a.input_forget_noop_record();
        assert!(a.input_undo());
        assert_eq!(a.input_text(), "");
    }

    #[test]
    fn selection_is_replaced_by_typing_and_cut_comes_back() {
        let mut a = app();
        a.set_input("one two three");
        a.input_move(super::Move::WordLeft, false);
        a.input_move(super::Move::WordLeft, true);
        assert_eq!(a.input_selection(), Some((4, 8)));
        assert!(a.input_cut());
        assert_eq!(a.input_text(), "one three");
        assert_eq!(a.cut_buffer, "two ");
        a.input_move(super::Move::TextEnd, false);
        a.input_type(' ');
        assert!(a.input_paste_cut_buffer());
        assert_eq!(a.input_text(), "one three two ");
        a.input_move(super::Move::Home, true);
        a.input_type('x');
        assert_eq!(a.input_text(), "x");
        assert!(a.input_selection().is_none());
    }
}
