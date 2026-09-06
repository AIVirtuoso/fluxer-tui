//! Word wrapping for styled spans. The message pane gives its rows a left
//! margin (the avatar column); ratatui's own wrapping would start every
//! continuation row at the left edge, under the avatar instead of under
//! the text, so rows are wrapped here first and the margin added after.

use ratatui::style::Style;
use ratatui::text::Span;
use unicode_width::UnicodeWidthChar;

struct Piece {
    text: String,
    style: Style,
    width: usize,
    space: bool,
}

/// Runs of blanks and runs of non-blanks, each with its span's style.
fn pieces(spans: &[Span<'static>]) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    for span in spans {
        let mut current: Option<Piece> = None;
        for ch in span.content.chars() {
            let w = UnicodeWidthChar::width(ch).unwrap_or(0);
            let space = ch.is_whitespace();
            match current.as_mut() {
                // zero-width marks stay with what they follow
                Some(p) if p.space == space || w == 0 => {
                    p.text.push(ch);
                    p.width += w;
                }
                _ => {
                    if let Some(p) = current.take() {
                        out.push(p);
                    }
                    current = Some(Piece {
                        text: ch.to_string(),
                        style: span.style,
                        width: w,
                        space,
                    });
                }
            }
        }
        if let Some(p) = current.take() {
            out.push(p);
        }
    }
    out
}

struct Rows {
    rows: Vec<Vec<(String, Style)>>,
    width: usize,
}

impl Rows {
    fn push(&mut self, text: &str, style: Style) {
        let row = self.rows.last_mut().expect("one row always open");
        match row.last_mut() {
            Some((t, s)) if *s == style => t.push_str(text),
            _ => row.push((text.to_string(), style)),
        }
    }

    /// Close the row: blanks at its end are dropped, a new row opens.
    fn newline(&mut self) {
        if let Some(row) = self.rows.last_mut() {
            while let Some((text, _)) = row.last_mut() {
                let trimmed = text.trim_end();
                if trimmed.len() == text.len() {
                    break;
                }
                text.truncate(trimmed.len());
                if text.is_empty() {
                    row.pop();
                }
            }
        }
        self.rows.push(Vec::new());
        self.width = 0;
    }
}

/// Break `spans` into rows at most `width` cells wide: at blanks where
/// possible, inside a word only when the word alone is wider than a row.
/// Blanks at a break are dropped; leading blanks of the first row stay.
pub fn wrap_spans(spans: &[Span<'static>], width: usize) -> Vec<Vec<Span<'static>>> {
    let width = width.max(1);
    let mut rows = Rows {
        rows: vec![Vec::new()],
        width: 0,
    };
    for p in pieces(spans) {
        if p.space {
            if rows.width == 0 && rows.rows.len() > 1 {
                continue;
            }
            if rows.width + p.width <= width {
                rows.push(&p.text, p.style);
                rows.width += p.width;
            } else {
                rows.newline();
            }
            continue;
        }
        if rows.width + p.width <= width {
            rows.push(&p.text, p.style);
            rows.width += p.width;
        } else if p.width <= width {
            rows.newline();
            rows.push(&p.text, p.style);
            rows.width = p.width;
        } else {
            for ch in p.text.chars() {
                let w = UnicodeWidthChar::width(ch).unwrap_or(0);
                if rows.width + w > width && rows.width > 0 {
                    rows.newline();
                }
                rows.push(&ch.to_string(), p.style);
                rows.width += w;
            }
        }
    }
    rows.rows
        .into_iter()
        .map(|row| row.into_iter().map(|(t, s)| Span::styled(t, s)).collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Modifier};

    fn text(rows: &[Vec<Span<'static>>]) -> Vec<String> {
        rows.iter()
            .map(|r| r.iter().map(|s| s.content.as_ref()).collect::<String>())
            .collect()
    }

    #[test]
    fn wraps_at_blanks_and_drops_the_blank_at_the_break() {
        let rows = wrap_spans(&[Span::raw("hello world foo bar")], 11);
        assert_eq!(text(&rows), ["hello world", "foo bar"]);
        let rows = wrap_spans(&[Span::raw("ab cd")], 2);
        assert_eq!(text(&rows), ["ab", "cd"]);
    }

    #[test]
    fn long_words_break_by_character_and_wide_characters_count_double() {
        let rows = wrap_spans(&[Span::raw("aaaaaaaaaaaa")], 5);
        assert_eq!(text(&rows), ["aaaaa", "aaaaa", "aa"]);
        let rows = wrap_spans(&[Span::raw("日本語テキスト")], 5);
        assert_eq!(text(&rows), ["日本", "語テ", "キス", "ト"]);
    }

    #[test]
    fn styles_survive_and_neighbours_merge() {
        let bold = Style::default().add_modifier(Modifier::BOLD);
        let red = Style::default().fg(Color::Red);
        let rows = wrap_spans(
            &[
                Span::styled("bold ", bold),
                Span::styled("words ", bold),
                Span::styled("red", red),
            ],
            20,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].len(), 2, "same-style spans merge");
        assert_eq!(rows[0][0].content, "bold words ");
        assert_eq!(rows[0][0].style, bold);
        assert_eq!(rows[0][1].style, red);
    }

    #[test]
    fn leading_blanks_stay_on_the_first_row_only() {
        let rows = wrap_spans(&[Span::raw("  code here and more")], 12);
        assert_eq!(text(&rows), ["  code here", "and more"]);
    }

    #[test]
    fn placeholders_are_never_split() {
        let rows = wrap_spans(&[Span::raw("x "), Span::raw("\u{2800}\u{2800}")], 3);
        assert_eq!(text(&rows), ["x", "\u{2800}\u{2800}"]);
        let rows = wrap_spans(&[], 10);
        assert_eq!(text(&rows), [""]);
    }
}
