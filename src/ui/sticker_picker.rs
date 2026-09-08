//! The sticker picker (Alt+S, `/sticker`): the stickers of every
//! community the client knows, the active one's first, with the sticker
//! under the cursor shown beside the list. The cursor moves with the vim
//! keys and `/` opens the search that filters the list.

use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(picker) = &app.sticker_picker else {
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

    let staged = app.pending_stickers.len();
    // the search as it stands: what is typed, with a caret while typing
    let filter = if picker.searching {
        format!("/{}\u{2588}", picker.query)
    } else if picker.query.is_empty() {
        "/ to search".to_string()
    } else {
        format!("/{}", picker.query)
    };
    let title = format!(
        " Send a sticker \u{2014} {} of {} staged  \u{2315} {filter} ",
        staged,
        crate::app::MAX_STICKERS_PER_MESSAGE,
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
            if picker.searching {
                "No sticker of that name or tag (Backspace edits the search)"
            } else {
                "No sticker of that name or tag (/ searches again)"
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
    let active_guild = app
        .guild_id_for_active_channel()
        .or_else(|| app.active_guild_id());
    for fi in start..end {
        let entry = &picker.entries[picker.filtered[fi]];
        let is_sel = fi == sel;
        let style = if is_sel {
            crate::ui::theme::highlight_style()
        } else {
            text
        };
        // A sticker of another community is named by it: sending one needs
        // the right to use external stickers.
        let side = if active_guild.as_deref() == Some(entry.guild_id.as_str()) {
            if entry.sticker.animated {
                "animated".to_string()
            } else {
                String::new()
            }
        } else {
            entry.guild_name.clone()
        };
        let name_w = (list_area.width as usize).saturating_sub(side.width() + 4);
        let name = fit(&entry.sticker.name, name_w);
        let pad = name_w.saturating_sub(name.width());
        items.push(ListItem::new(Line::from(vec![
            Span::styled(format!(" {name}{}", " ".repeat(pad)), style),
            Span::styled(
                format!(" {} ", fit(&side, 18)),
                if is_sel { style } else { muted },
            ),
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
        if let Some(entry) = picker.current() {
            lines.push(Line::from(Span::styled(
                fit(&entry.sticker.name, side_inner.width as usize),
                accent,
            )));
            lines.push(Line::from(Span::styled(
                fit(&entry.guild_name, side_inner.width as usize),
                dim,
            )));
            let description = entry
                .sticker
                .description
                .as_deref()
                .map(str::trim)
                .filter(|d| !d.is_empty());
            match description {
                Some(d) => lines.push(Line::from(Span::styled(
                    fit(d, side_inner.width as usize),
                    dim,
                ))),
                None if !entry.sticker.tags.is_empty() => lines.push(Line::from(Span::styled(
                    fit(&entry.sticker.tags.join(", "), side_inner.width as usize),
                    dim,
                ))),
                None => lines.push(Line::from("")),
            }
            lines.push(Line::from(Span::styled(
                "Enter puts it on the message",
                muted,
            )));
            // The picture goes under the four lines of text, and no
            // taller than a block the media overlay can draw.
            let max = (
                side_inner.width.saturating_sub(1),
                side_inner
                    .height
                    .saturating_sub(5)
                    .min(crate::media::BLOCK_MAX_ROWS),
            );
            if let Some(slot) = app.sticker_slot(&entry.sticker.id, entry.sticker.animated, max) {
                let rect = Rect::new(side_inner.x, side_inner.y + 5, slot.cols, slot.rows);
                thumb = Some((slot, rect));
            }
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

    let hint = Paragraph::new(Line::from(Span::styled(
        if picker.searching {
            "type to filter \u{00B7} Enter keeps it \u{00B7} Esc cancels the search \u{00B7} Backspace edits"
        } else {
            "j/k \u{2191}\u{2193} move \u{00B7} g/G top/bottom \u{00B7} / search \u{00B7} Enter stage it \u{00B7} q close"
        },
        muted,
    )))
    .alignment(Alignment::Center);
    frame.render_widget(hint, foot);
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

#[cfg(test)]
mod tests {
    use crate::api::types::GuildResponse;
    use crate::app::{App, Picture, PictureFrames, ServerSelection};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::sync::Arc;
    use std::time::Duration;

    /// A community with one sticker, on a terminal that draws pictures
    /// (the console renderer, whose cells are the user's 11x25 px).
    fn app_with_a_sticker() -> App {
        let mut app = App::new(
            Default::default(),
            Default::default(),
            None,
            Vec::new(),
            Vec::new(),
            ServerSelection::Guild("g1".to_string()),
            None,
            Default::default(),
        );
        app.guilds.push(GuildResponse {
            id: "g1".to_string(),
            name: "Lab".to_string(),
            ..Default::default()
        });
        app.set_guild_stickers(
            "g1",
            vec![crate::api::types::GuildStickerResponse {
                id: "77".to_string(),
                name: "shipit".to_string(),
                ..Default::default()
            }],
        );
        app.pixel_mode = true;
        app.cell_px = (11, 25);
        app
    }

    /// The picker asks for a block the overlay can draw, and the picture
    /// lands on it. A block over 16 rows tall marks every further row as
    /// the 16th, and the overlay then draws nothing at all.
    #[test]
    fn the_selected_stickers_picture_is_drawn_beside_the_list() {
        let mut app = app_with_a_sticker();
        app.open_sticker_picker("");
        let mut terminal = Terminal::new(TestBackend::new(114, 54)).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();

        let slot = app
            .media_slots
            .borrow()
            .iter()
            .find(|s| s.url.contains("/stickers/77.webp"))
            .cloned()
            .expect("the preview claimed no block");
        assert!(
            slot.rows > 1 && slot.rows <= crate::media::BLOCK_MAX_ROWS,
            "a block of {} rows cannot be drawn",
            slot.rows
        );

        // the picture arrives: the next frame puts it on those cells
        let px = crate::media::block_px(slot.cols, slot.rows, app.cell_px);
        let image = image::RgbaImage::from_pixel(px.0, px.1, image::Rgba([1, 2, 3, 255]));
        assert!(app.media.start(&slot.key));
        app.set_media_frames(
            slot.key.clone(),
            Some(PictureFrames::new(
                vec![Picture::Pixels(Arc::new(image))],
                vec![Duration::ZERO],
            )),
            (px.0 * px.1 * 4) as usize,
        );
        terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();

        let placed = app.pixel_placements.borrow();
        assert_eq!(placed.len(), 1, "the sticker's picture was not drawn");
        assert_eq!(placed[0].area.width, slot.cols);
        assert_eq!(placed[0].area.height, slot.rows);
    }
}
