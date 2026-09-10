//! The file picker (`/attach` alone, Ctrl+F): a directory's entries on the
//! left, filtered as the user types; what is under the cursor on the
//! right, with a preview when it is an image or a video.

use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(picker) = &app.file_picker else {
        return;
    };
    let accent = Style::default()
        .fg(crate::ui::theme::accent())
        .add_modifier(Modifier::BOLD);
    let dim = crate::ui::theme::dim_style();
    let muted = crate::ui::theme::muted_style();
    let text = Style::default().fg(crate::ui::theme::text());

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(8),
            Constraint::Percentage(84),
            Constraint::Percentage(8),
        ])
        .split(area);
    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(6),
            Constraint::Percentage(88),
            Constraint::Percentage(6),
        ])
        .split(outer[1]);
    let popup = mid[1];
    frame.render_widget(Clear, popup);

    let dir = shorten_home(&picker.dir);
    let title = format!(
        " Attach a file \u{2014} {dir}  \u{2315} {} ",
        if picker.query.is_empty() {
            "type to filter"
        } else {
            picker.query.as_str()
        }
    );
    let block = Block::default()
        .title(Line::from(Span::styled(
            fit(&title, popup.width as usize),
            accent,
        )))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(crate::ui::theme::accent_dim()));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.height < 2 || inner.width < 20 {
        return;
    }
    let body = Rect {
        height: inner.height - 1,
        ..inner
    };
    let foot = Rect {
        y: inner.y + inner.height - 1,
        height: 1,
        ..inner
    };
    let show_preview = body.width >= 50;
    let cols = if show_preview {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(body)
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(100)])
            .split(body)
    };
    let list_area = cols[0];

    let total = picker.filtered.len();
    let mut items: Vec<ListItem> = Vec::new();
    if total == 0 {
        items.push(ListItem::new(Line::from(Span::styled(
            if picker.entries.is_empty() {
                "Empty directory (Backspace goes up)"
            } else {
                "No match (Backspace edits the filter)"
            },
            dim,
        ))));
    }
    let visible = list_area.height.max(1) as usize;
    let sel = picker.selected.min(total.saturating_sub(1));
    let start = sel
        .saturating_sub(visible / 2)
        .min(total.saturating_sub(visible));
    let end = (start + visible).min(total);
    for fi in start..end {
        let entry = &picker.entries[picker.filtered[fi]];
        let is_sel = fi == sel;
        let style = if is_sel {
            crate::ui::theme::highlight_style()
        } else if entry.is_dir {
            accent
        } else {
            text
        };
        let size = if entry.is_dir {
            String::new()
        } else {
            crate::media::human_size(entry.size as usize)
        };
        let name_w = (list_area.width as usize).saturating_sub(size.width() + 4);
        let name = if entry.is_dir {
            format!("{}/", entry.name)
        } else {
            entry.name.clone()
        };
        let name = fit(&name, name_w);
        let pad = name_w.saturating_sub(name.width());
        items.push(ListItem::new(Line::from(vec![
            Span::styled(format!(" {name}{}", " ".repeat(pad)), style),
            Span::styled(format!(" {size} "), if is_sel { style } else { muted }),
        ])));
    }
    frame.render_widget(List::new(items), list_area);

    if show_preview {
        let side = cols[1];
        let side_inner = Rect {
            x: side.x + 1,
            width: side.width.saturating_sub(1),
            ..side
        };
        let mut lines: Vec<Line<'static>> = Vec::new();
        let mut thumb: Option<(crate::app::MediaSlot, Rect)> = None;
        match picker.current() {
            Some(entry) if entry.is_dir => {
                lines.push(Line::from(Span::styled(
                    fit(&entry.name, side_inner.width as usize),
                    accent,
                )));
                lines.push(Line::from(Span::styled(
                    "directory \u{00B7} Enter opens",
                    dim,
                )));
            }
            Some(entry) => {
                lines.push(Line::from(Span::styled(
                    fit(&entry.name, side_inner.width as usize),
                    accent,
                )));
                let ext = std::path::Path::new(&entry.name)
                    .extension()
                    .map(|e| e.to_string_lossy().to_string())
                    .unwrap_or_default();
                let kind = crate::media::content_type_for_extension(&ext);
                lines.push(Line::from(Span::styled(
                    format!(
                        "{} \u{00B7} {}",
                        crate::media::human_size(entry.size as usize),
                        kind
                    ),
                    dim,
                )));
                lines.push(Line::from(Span::styled("Enter attaches it", muted)));
                let max = (
                    side_inner.width.saturating_sub(1),
                    side_inner.height.saturating_sub(4),
                );
                if let Some(slot) = app.file_preview_slot(&entry.path, max) {
                    lines.push(Line::from(""));
                    let rect = Rect::new(side_inner.x, side_inner.y + 4, slot.cols, slot.rows);
                    thumb = Some((slot, rect));
                } else if crate::media::is_video("", &entry.name) {
                    lines.push(Line::from(Span::styled(
                        "(preview needs ffmpeg and a terminal that draws pictures)",
                        muted,
                    )));
                }
            }
            None => {}
        }
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), side_inner);
        if let Some((slot, rect)) = thumb {
            let k = app.register_media_slot(slot);
            let buf = frame.buffer_mut();
            for r in 0..rect.height {
                let style = crate::app::media_marker_style(k, r);
                for c in 0..rect.width {
                    if let Some(cell) = buf.cell_mut((rect.x + c, rect.y + r)) {
                        cell.set_symbol("\u{2800}").set_style(style);
                    }
                }
            }
        }
    }

    crate::ui::footer::render(
        frame,
        foot,
        app,
        "↑↓ move · Enter open/attach · ← up a folder · type to filter · Esc close",
    );
}

fn shorten_home(path: &std::path::Path) -> String {
    let s = path.display().to_string();
    match dirs::home_dir() {
        Some(home) if path.starts_with(&home) => {
            format!("~{}", &s[home.display().to_string().len()..])
        }
        _ => s,
    }
}

/// `s` cut to `width` cells, with an ellipsis when something was cut.
fn fit(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in s.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('\u{2026}');
    out
}
