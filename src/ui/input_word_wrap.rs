use std::collections::VecDeque;
use std::mem;

use ratatui::layout::Alignment;
use ratatui::style::Style;
use ratatui::text::{Line, Span, StyledGrapheme, Text};
use ratatui::widgets::{Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

const NBSP: &str = "\u{00a0}";
const ZWSP: &str = "\u{200b}";

#[inline]
fn grapheme_is_whitespace(g: &StyledGrapheme<'_>) -> bool {
    let symbol = g.symbol;
    symbol == ZWSP || symbol.chars().all(char::is_whitespace) && symbol != NBSP
}

struct WrappedLine<'lend, 'text> {
    #[allow(dead_code)]
    line: &'lend [StyledGrapheme<'text>],
    width: u16,
    #[allow(dead_code)]
    alignment: Alignment,
}

#[derive(Debug, Default, Clone)]
struct WordWrapper<'a, O, I>
where
    O: Iterator<Item = (I, Alignment)>,
    I: Iterator<Item = StyledGrapheme<'a>>,
{
    input_lines: O,
    max_line_width: u16,
    wrapped_lines: VecDeque<Vec<StyledGrapheme<'a>>>,
    current_alignment: Alignment,
    current_line: Vec<StyledGrapheme<'a>>,
    trim: bool,
    pending_word: Vec<StyledGrapheme<'a>>,
    pending_whitespace: VecDeque<StyledGrapheme<'a>>,
    pending_line_pool: Vec<Vec<StyledGrapheme<'a>>>,
}

impl<'a, O, I> WordWrapper<'a, O, I>
where
    O: Iterator<Item = (I, Alignment)>,
    I: Iterator<Item = StyledGrapheme<'a>>,
{
    const fn new(lines: O, max_line_width: u16, trim: bool) -> Self {
        Self {
            input_lines: lines,
            max_line_width,
            wrapped_lines: VecDeque::new(),
            current_alignment: Alignment::Left,
            current_line: vec![],
            trim,
            pending_word: Vec::new(),
            pending_line_pool: Vec::new(),
            pending_whitespace: VecDeque::new(),
        }
    }

    fn process_input(&mut self, line_symbols: impl IntoIterator<Item = StyledGrapheme<'a>>) {
        let mut pending_line = self.pending_line_pool.pop().unwrap_or_default();
        let mut line_width = 0;
        let mut word_width = 0;
        let mut whitespace_width = 0;
        let mut non_whitespace_previous = false;

        self.pending_word.clear();
        self.pending_whitespace.clear();
        pending_line.clear();

        for grapheme in line_symbols {
            let is_whitespace = grapheme_is_whitespace(&grapheme);
            let symbol_width = grapheme.symbol.width() as u16;

            if symbol_width > self.max_line_width {
                continue;
            }

            let word_found = non_whitespace_previous && is_whitespace;
            let trimmed_overflow = pending_line.is_empty()
                && self.trim
                && word_width + symbol_width > self.max_line_width;
            let whitespace_overflow = pending_line.is_empty()
                && self.trim
                && whitespace_width + symbol_width > self.max_line_width;
            let untrimmed_overflow = pending_line.is_empty()
                && !self.trim
                && word_width + whitespace_width + symbol_width > self.max_line_width;

            if word_found || trimmed_overflow || whitespace_overflow || untrimmed_overflow {
                if !pending_line.is_empty() || !self.trim {
                    pending_line.extend(self.pending_whitespace.drain(..));
                    line_width += whitespace_width;
                }

                pending_line.append(&mut self.pending_word);
                line_width += word_width;

                self.pending_whitespace.clear();
                whitespace_width = 0;
                word_width = 0;
            }

            let line_full = line_width >= self.max_line_width;
            let pending_word_overflow = symbol_width > 0
                && line_width + whitespace_width + word_width >= self.max_line_width;

            if line_full || pending_word_overflow {
                let mut remaining_width = u16::saturating_sub(self.max_line_width, line_width);

                self.wrapped_lines.push_back(mem::take(&mut pending_line));
                line_width = 0;

                while let Some(grapheme) = self.pending_whitespace.front() {
                    let width = grapheme.symbol.width() as u16;

                    if width > remaining_width {
                        break;
                    }

                    whitespace_width -= width;
                    remaining_width -= width;
                    self.pending_whitespace.pop_front();
                }

                if is_whitespace && self.pending_whitespace.is_empty() {
                    continue;
                }
            }

            if is_whitespace {
                whitespace_width += symbol_width;
                self.pending_whitespace.push_back(grapheme);
            } else {
                word_width += symbol_width;
                self.pending_word.push(grapheme);
            }

            non_whitespace_previous = !is_whitespace;
        }

        if pending_line.is_empty()
            && self.pending_word.is_empty()
            && !self.pending_whitespace.is_empty()
        {
            self.wrapped_lines.push_back(vec![]);
        }
        if !pending_line.is_empty() || !self.trim {
            pending_line.extend(self.pending_whitespace.drain(..));
        }
        pending_line.append(&mut self.pending_word);

        if !pending_line.is_empty() {
            self.wrapped_lines.push_back(pending_line);
        } else if pending_line.capacity() > 0 {
            self.pending_line_pool.push(pending_line);
        }
        if self.wrapped_lines.is_empty() {
            self.wrapped_lines.push_back(vec![]);
        }
    }

    fn replace_current_line(&mut self, line: Vec<StyledGrapheme<'a>>) {
        let cache = mem::replace(&mut self.current_line, line);
        if cache.capacity() > 0 {
            self.pending_line_pool.push(cache);
        }
    }

    fn next_line<'lend>(&'lend mut self) -> Option<WrappedLine<'lend, 'a>> {
        if self.max_line_width == 0 {
            return None;
        }

        loop {
            if let Some(line) = self.wrapped_lines.pop_front() {
                let line_width = line
                    .iter()
                    .map(|grapheme| grapheme.symbol.width() as u16)
                    .sum();

                self.replace_current_line(line);
                return Some(WrappedLine {
                    line: &self.current_line,
                    width: line_width,
                    alignment: self.current_alignment,
                });
            }

            let (line_symbols, line_alignment) = self.input_lines.next()?;
            self.current_alignment = line_alignment;
            self.process_input(line_symbols);
        }
    }
}

fn input_text_lines(input: &str, span_style: Style) -> Text<'static> {
    let lines: Vec<Line> = input
        .split('\n')
        .map(|l| Line::from(Span::styled(l.to_string(), span_style)))
        .collect();
    Text::from(lines)
}

pub fn wrapped_row_count(input: &str, inner_width: u16, span_style: Style) -> u16 {
    let w = inner_width.max(1);
    if input.is_empty() {
        return 1;
    }
    let text = input_text_lines(input, span_style);
    let n = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .line_count(w);
    n.max(1) as u16
}

/// Where the end of the text lands; the reference the cursor test checks against.
#[cfg(test)]
pub fn eol_cursor_col_row(input: &str, inner_width: u16, span_style: Style) -> (u16, u16) {
    let w = inner_width.max(1);
    let text = input_text_lines(input, span_style);
    let base = Style::default();
    let styled = text.iter().map(|line| {
        let graphemes = line
            .spans
            .iter()
            .flat_map(|span| span.styled_graphemes(base));
        let alignment = line.alignment.unwrap_or(Alignment::Left);
        (graphemes, alignment)
    });
    let mut ww = WordWrapper::new(styled, w, false);
    let mut total_rows: u16 = 0;
    let mut last_w: u16 = 0;
    while let Some(wl) = ww.next_line() {
        last_w = wl.width;
        total_rows = total_rows.saturating_add(1);
    }
    if total_rows == 0 {
        (0, 0)
    } else {
        (last_w, total_rows - 1)
    }
}

/// Rows a single source line takes when wrapped.
fn rows_of_line(line: &str, w: u16, span_style: Style) -> u16 {
    let text = input_text_lines(line, span_style);
    let base = Style::default();
    let styled = text.iter().map(|line| {
        let graphemes = line
            .spans
            .iter()
            .flat_map(|span| span.styled_graphemes(base));
        (graphemes, Alignment::Left)
    });
    let mut ww = WordWrapper::new(styled, w, false);
    let mut n = 0u16;
    while ww.next_line().is_some() {
        n = n.saturating_add(1);
    }
    n.max(1)
}

/// Where the cursor sits in the wrapped display of `input` when the text
/// before it is `head` (both already flattened for display): column and
/// row inside the box. A cursor at the very end of a full row moves to
/// the start of the next row when there is one, as the next typed
/// character would land there.
pub fn cursor_col_row(input: &str, head: &str, inner_width: u16, span_style: Style) -> (u16, u16) {
    let w = inner_width.max(1);
    let line_idx = head.matches('\n').count();
    let head_line = head.rsplit('\n').next().unwrap_or("");
    let base = Style::default();
    let target = Span::raw(head_line).styled_graphemes(base).count();

    let mut row: u16 = 0;
    let mut lines = input.split('\n');
    for _ in 0..line_idx {
        let Some(line) = lines.next() else {
            break;
        };
        row = row.saturating_add(rows_of_line(line, w, span_style));
    }
    let Some(line) = lines.next() else {
        return (0, row);
    };
    let source_span = Span::raw(line);
    let source: Vec<StyledGrapheme<'_>> = source_span.styled_graphemes(base).collect();
    let text = input_text_lines(line, span_style);
    let styled = text.iter().map(|line| {
        let graphemes = line
            .spans
            .iter()
            .flat_map(|span| span.styled_graphemes(base));
        (graphemes, Alignment::Left)
    });
    let mut ww = WordWrapper::new(styled, w, false);
    let mut si = 0usize;
    let mut rows_in_line: Vec<(u16, Option<u16>)> = Vec::new(); // (width, cursor col if on this row)
    while let Some(wl) = ww.next_line() {
        let mut col: u16 = 0;
        let mut hit: Option<u16> = None;
        for g in wl.line.iter() {
            while si < source.len() && source[si].symbol != g.symbol {
                // whitespace the wrapper dropped at a break
                if si == target && hit.is_none() {
                    hit = Some(col);
                }
                si += 1;
            }
            if si == target && hit.is_none() {
                hit = Some(col);
            }
            col = col.saturating_add(g.symbol.width() as u16);
            si += 1;
        }
        if si >= target && hit.is_none() {
            hit = Some(col);
        }
        rows_in_line.push((wl.width, hit));
    }
    let n = rows_in_line.len();
    for (i, (_, hit)) in rows_in_line.iter().enumerate() {
        if let Some(col) = hit {
            if *col >= w && i + 1 < n {
                return (0, row.saturating_add(i as u16 + 1));
            }
            return (*col, row.saturating_add(i as u16));
        }
    }
    let last = rows_in_line.last().map(|r| r.0).unwrap_or(0);
    (last, row.saturating_add(n.saturating_sub(1) as u16))
}

#[cfg(test)]
mod cursor_tests {
    use super::*;

    fn at(input: &str, head: &str, w: u16) -> (u16, u16) {
        cursor_col_row(input, head, w, Style::default())
    }

    #[test]
    fn cursor_inside_one_row() {
        assert_eq!(at("hello", "", 20), (0, 0));
        assert_eq!(at("hello", "hel", 20), (3, 0));
        assert_eq!(at("hello", "hello", 20), (5, 0));
        assert_eq!(at("", "", 20), (0, 0));
    }

    #[test]
    fn cursor_follows_the_wrap() {
        // "hello world" at width 8 wraps to "hello" / "world": the wrapper
        // drops the space at the break, so a cursor after it sits where
        // the next character would go, at the start of the second row
        assert_eq!(at("hello world", "hello", 8), (5, 0));
        assert_eq!(at("hello world", "hello ", 8), (0, 1));
        assert_eq!(at("hello world", "hello w", 8), (1, 1));
        assert_eq!(at("hello world", "hello world", 8), (5, 1));
        // the end of the text agrees with the end-of-line helper
        let text = "the quick brown fox jumps over the lazy dog";
        assert_eq!(
            at(text, text, 10),
            eol_cursor_col_row(text, 10, Style::default())
        );
    }

    #[test]
    fn cursor_on_later_lines_counts_the_rows_above() {
        assert_eq!(
            at(
                "ab
cd", "ab
", 20
            ),
            (0, 1)
        );
        assert_eq!(
            at(
                "ab
cd", "ab
c", 20
            ),
            (1, 1)
        );
        assert_eq!(
            at(
                "ab

cd", "ab

", 20
            ),
            (0, 2)
        );
        // a wrapped first line pushes the second one down
        assert_eq!(
            at(
                "hello world
x",
                "hello world
x",
                8
            ),
            (1, 2)
        );
    }

    #[test]
    fn wide_characters_count_their_cells() {
        assert_eq!(at("日本語", "日本", 20), (4, 0));
    }
}
