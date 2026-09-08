use crate::app::{App, Focus, THUMB_ROWS};
use crate::ui::input_word_wrap;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_width::UnicodeWidthStr;

/// Rows the staged files take above the text: their names, under their
/// thumbnails when any is a picture or a video the terminal can draw.
pub fn attachment_strip_rows(app: &App) -> u16 {
    if app.pending_attachments.is_empty() {
        return 0;
    }
    let thumbs = app
        .pending_attachments
        .iter()
        .any(|a| app.staged_thumbnail_slot(a).is_some());
    if thumbs { THUMB_ROWS + 1 } else { 1 }
}

/// The strip: one card per staged file, side by side, `width` cells wide.
/// A card is its thumbnail's marker cells (the media overlay draws the
/// picture there) over its name and size; files without a picture get
/// a paper clip. Cards that do not fit are counted at the end.
fn attachment_strip(app: &App, width: u16) -> Vec<Line<'static>> {
    let rows = attachment_strip_rows(app) as usize;
    if rows == 0 {
        return Vec::new();
    }
    let thumb_rows = rows - 1;
    let dim = crate::ui::theme::dim_style();
    let muted = crate::ui::theme::muted_style();
    let mut lines: Vec<Vec<Span<'static>>> = vec![Vec::new(); rows];
    let mut x = 0usize;
    let mut shown = 0usize;
    for a in &app.pending_attachments {
        let slot = app.staged_thumbnail_slot(a);
        let (cols, prows) = slot
            .as_ref()
            .map(|s| (s.cols as usize, s.rows as usize))
            .unwrap_or((0, 0));
        let label = format!("{} {}", a.filename, a.size_label());
        let card_w = cols.max(label.width().min(20)).max(6);
        if x + card_w > width as usize {
            break;
        }
        let gap = if shown > 0 { 2 } else { 0 };
        let k = slot.map(|s| app.register_media_slot(s));
        for (r, line) in lines.iter_mut().enumerate().take(thumb_rows) {
            line.push(Span::raw(" ".repeat(gap)));
            match k {
                Some(k) if r < prows => {
                    line.push(Span::styled(
                        "\u{2800}".repeat(cols),
                        crate::app::media_marker_style(k, r as u16),
                    ));
                    line.push(Span::raw(" ".repeat(card_w - cols)));
                }
                None if r == thumb_rows / 2 => {
                    let mark = "\u{1F4CE}";
                    let pad = card_w.saturating_sub(2) / 2;
                    line.push(Span::raw(" ".repeat(pad)));
                    line.push(Span::styled(mark, muted));
                    line.push(Span::raw(" ".repeat(card_w.saturating_sub(pad + 2))));
                }
                _ => line.push(Span::raw(" ".repeat(card_w))),
            }
        }
        let name = fit(&label, card_w);
        let pad = card_w.saturating_sub(name.width());
        let name_line = &mut lines[rows - 1];
        name_line.push(Span::raw(" ".repeat(gap)));
        name_line.push(Span::styled(name, dim));
        name_line.push(Span::raw(" ".repeat(pad)));
        x += gap + card_w;
        shown += 1;
    }
    let left = app.pending_attachments.len().saturating_sub(shown);
    if left > 0 {
        lines[rows - 1].push(Span::styled(format!("  +{left} more"), muted));
    }
    lines.into_iter().map(Line::from).collect()
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

fn input_span_style() -> Style {
    Style::default().fg(crate::ui::theme::text())
}

fn typing_line_with_dots(phrase: &str, dots: &str) -> String {
    let base = phrase.strip_suffix("...").unwrap_or(phrase);
    format!("{base}{dots}")
}

pub fn input_display_row_count(app: &App, inner_width: u16) -> u16 {
    let can_type = app.active_channel_is_text() && app.can_send_in_active_channel();
    if !can_type {
        return 1;
    }
    let strip = attachment_strip_rows(app);
    if !app.input_is_empty() {
        return input_word_wrap::wrapped_row_count(
            &app.input_display_plain(),
            inner_width,
            input_span_style(),
        )
        .saturating_add(strip);
    }
    let mut n = 1u16;
    if app.others_typing_phrase().is_some() {
        n = n.saturating_add(1);
    }
    n.saturating_add(strip)
}

pub fn render(frame: &mut Frame, area: Rect, app: &App) -> Option<(u16, u16)> {
    let can_type = app.active_channel_is_text() && app.can_send_in_active_channel();
    let no_perms = app.active_channel_is_text() && !app.can_send_in_active_channel();
    let voice_only = app.active_channel_is_voice();

    let title = if voice_only {
        "Input (voice not supported)"
    } else if no_perms {
        "Input (no permission)"
    } else if app.edit_target.is_some() {
        "Edit message"
    } else if app.forward_mode {
        "Forward (select channel, Enter to send)"
    } else if app.reply_to.is_some() {
        "Reply"
    } else if can_type {
        "Input"
    } else {
        "Input (disabled)"
    };
    let title: String = if app.pending_attachments.is_empty() {
        title.to_string()
    } else {
        format!("{title} · {}", app.attachment_summary())
    };

    let placeholder: Option<String> = if voice_only || no_perms || !can_type {
        None
    } else if app.input_is_empty() {
        Some(if let Some(ref reply) = app.reply_to {
            if app.forward_mode {
                format!(
                    "Forward from {} - optional note, Enter to send",
                    reply.author_name
                )
            } else {
                format!("Replying to {}...", reply.author_name)
            }
        } else {
            "Type a message…  ( / for commands )".to_string()
        })
    } else {
        None
    };

    let (content, style) = if voice_only {
        (
            "This client cannot join or use voice - text input is disabled here.".to_string(),
            crate::ui::theme::muted_style(),
        )
    } else if no_perms {
        (
            "You do not have permission to send messages here.".to_string(),
            crate::ui::theme::muted_style(),
        )
    } else if can_type && !app.input_is_empty() {
        (String::new(), Style::default().fg(crate::ui::theme::text()))
    } else if can_type {
        (
            placeholder.clone().unwrap_or_default(),
            crate::ui::theme::muted_style(),
        )
    } else {
        (
            "Select a text channel to chat.".to_string(),
            crate::ui::theme::muted_style(),
        )
    };

    let others_typing = app.others_typing_phrase();
    let typing_dots: &'static str = if others_typing.is_some() {
        match app.input_bar_anim_phase % 4 {
            0 => "",
            1 => ".",
            2 => "..",
            _ => "...",
        }
    } else {
        ""
    };

    let title_line = Line::from(Span::styled(
        format!(" {title}"),
        Style::default()
            .add_modifier(Modifier::BOLD)
            .patch(crate::ui::theme::dim_style()),
    ));

    let focused = app.focus == Focus::Input;
    let mut blk = Block::default()
        .title(title_line)
        .borders(Borders::ALL)
        .border_style(crate::ui::theme::focused_border(focused))
        .style(Style::default().bg(crate::ui::theme::bg_secondary()));

    if can_type && !app.input_is_empty() {
        let char_count = app.input_char_count();
        let max_chars = 2000;
        let count_str = format!(" {char_count}/{max_chars} ");
        let count_style = if char_count > max_chars {
            Style::default()
                .fg(crate::ui::theme::danger())
                .add_modifier(Modifier::BOLD)
        } else if char_count > max_chars - 100 {
            Style::default()
                .fg(ratatui::style::Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            crate::ui::theme::muted_style()
        };

        let right_title = Line::from(Span::styled(count_str, count_style))
            .alignment(ratatui::layout::Alignment::Right);
        blk = blk.title(right_title);
    }

    // The staged files sit above the text. The strip is drawn as it is,
    // never through the word wrapper: ratatui's wrapper makes two rows of
    // a line that is only spaces, and a thumbnail card has such rows under
    // its picture, which pushed the text below the box.
    let inner_w = area.width.saturating_sub(2).max(1);
    let strip: Vec<Line<'static>> = if can_type {
        attachment_strip(app, inner_w)
    } else {
        Vec::new()
    };
    let strip_rows = strip.len() as u16;
    let mut lines: Vec<Line<'static>> = Vec::new();
    if can_type && !app.input_is_empty() {
        lines.extend(app.input_display(true));
    } else if can_type
        && app.input_is_empty()
        && let (Some(phrase), Some(ph)) = (others_typing.as_ref(), placeholder.as_ref())
    {
        let typing_style = Style::default()
            .fg(crate::ui::theme::typing_others())
            .add_modifier(Modifier::ITALIC);
        let typing_text = typing_line_with_dots(phrase, typing_dots);
        lines.push(Line::from(Span::styled(typing_text, typing_style)));
        lines.push(Line::from(Span::styled(ph.clone(), style)));
    } else {
        lines.push(Line::from(Span::styled(content, style)));
    }
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    let strip_h = strip_rows.min(inner.height);
    if strip_h > 0 {
        let strip_area = Rect::new(inner.x, inner.y, inner.width, strip_h);
        frame.render_widget(Paragraph::new(Text::from(strip)), strip_area);
    }
    let text_area = Rect::new(
        inner.x,
        inner.y.saturating_add(strip_h),
        inner.width,
        inner.height.saturating_sub(strip_h),
    );
    // The cursor row decides how far the text scrolls when it is taller
    // than the box: the row with the cursor is always in view.
    let cursor = if focused && can_type && !app.input_is_empty() {
        Some(input_word_wrap::cursor_col_row(
            &app.input_display_plain(),
            &app.input_head_display_plain(),
            inner_w,
            input_span_style(),
        ))
    } else {
        None
    };
    let visible_rows = text_area.height.max(1);
    let scroll = match cursor {
        Some((_, row)) if row >= visible_rows => row - visible_rows + 1,
        _ => 0,
    };
    let paragraph = Paragraph::new(Text::from(lines))
        .wrap(ratatui::widgets::Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(paragraph, text_area);

    if let Some((col, row)) = cursor {
        let max_x = area.x + area.width.saturating_sub(2);
        let x = (area.x + 1 + col).min(max_x);
        let y = area.y + 1 + strip_rows + (row - scroll);
        Some((x, y))
    } else if focused && can_type {
        let extra = if app.input_is_empty() && app.others_typing_phrase().is_some() {
            1u16
        } else {
            0
        };
        Some((area.x + 1, area.y + 1 + strip_rows + extra))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::{ChannelResponse, UserPartialResponse, UserPrivateResponse};
    use crate::app::ServerSelection;

    fn dm_app() -> App {
        let me = UserPrivateResponse {
            id: "me".into(),
            ..Default::default()
        };
        let channel = ChannelResponse {
            id: "c1".into(),
            kind: 1,
            recipients: vec![UserPartialResponse {
                id: "o".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        App::new(
            Default::default(),
            me,
            None,
            Vec::new(),
            vec![channel],
            ServerSelection::DirectMessages,
            Some("c1".into()),
            Default::default(),
        )
    }

    #[test]
    fn staged_files_take_a_strip_above_the_text() {
        let mut app = dm_app();
        assert!(app.active_channel_is_text() && app.can_send_in_active_channel());
        assert_eq!(input_display_row_count(&app, 60), 1);
        app.pending_attachments
            .push(crate::media::StagedAttachment::new(
                "notes.txt".into(),
                "text/plain".into(),
                b"hi".to_vec(),
            ));
        // names only where pictures cannot be drawn
        assert_eq!(attachment_strip_rows(&app), 1);
        assert_eq!(input_display_row_count(&app, 60), 2);
        let lines = attachment_strip(&app, 60);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].to_string().contains("notes.txt 2 B"));

        let mut png = Vec::new();
        image::RgbaImage::from_pixel(40, 40, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        app.pending_attachments
            .push(crate::media::StagedAttachment::new(
                "shot.png".into(),
                "image/png".into(),
                png,
            ));
        app.pixel_mode = true;
        app.cell_px = (10, 20);
        assert_eq!(attachment_strip_rows(&app), THUMB_ROWS + 1);
        app.input = "hello".into();
        assert_eq!(input_display_row_count(&app, 60), THUMB_ROWS + 2);
        let lines = attachment_strip(&app, 60);
        assert_eq!(lines.len() as u16, THUMB_ROWS + 1);
        // the picture's marker cells sit on the thumbnail rows
        let slots = app.media_slots.borrow();
        assert_eq!(slots.len(), 1);
        let marked = lines[0]
            .spans
            .iter()
            .filter(|s| crate::app::media_marker(s.style) == Some((0, 0)))
            .count();
        assert_eq!(marked, 1, "{:?}", lines[0]);
        assert!(lines[THUMB_ROWS as usize].to_string().contains("shot.png"));
    }

    #[test]
    fn the_text_stays_in_the_box_under_a_thumbnail() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut app = dm_app();
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(40, 40, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        app.pending_attachments
            .push(crate::media::StagedAttachment::new(
                "shot.png".into(),
                "image/png".into(),
                png,
            ));
        app.pixel_mode = true;
        app.cell_px = (11, 25);
        app.input = "hello there".into();
        app.focus = Focus::Input;
        let mut terminal = Terminal::new(TestBackend::new(114, 54)).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let row = |y: u16| -> String {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect()
        };
        let rows: Vec<String> = (0..buf.area.height).map(row).collect();
        let bottom = buf.area.height - 1;
        assert!(
            rows[bottom as usize].starts_with('└'),
            "{:?}",
            rows[bottom as usize]
        );
        // the 40x40 picture is two rows tall in 11x25 cells: its marker
        // rows, two blank rows, the name, then the text, all inside
        assert!(
            rows[(bottom - 1) as usize].contains("hello there"),
            "{rows:#?}"
        );
        assert!(
            rows[(bottom - 2) as usize].contains("shot.png"),
            "{rows:#?}"
        );
        assert!(
            rows[(bottom - 5) as usize].contains('\u{2800}'),
            "{rows:#?}"
        );
        assert!(
            rows[(bottom - 6) as usize].contains('\u{2800}'),
            "{rows:#?}"
        );
        assert!(rows[(bottom - 7) as usize].contains("Input"), "{rows:#?}");
    }

    #[test]
    fn cards_that_do_not_fit_are_counted() {
        let mut app = dm_app();
        for i in 0..6 {
            app.pending_attachments
                .push(crate::media::StagedAttachment::new(
                    format!("a-rather-long-file-name-{i}.bin"),
                    "application/octet-stream".into(),
                    vec![0; 10],
                ));
        }
        let lines = attachment_strip(&app, 50);
        assert!(
            lines[0].to_string().contains("more"),
            "{:?}",
            lines[0].to_string()
        );
    }
}
