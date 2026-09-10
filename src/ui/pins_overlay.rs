//! The pinned messages of the open channel, newest pin first, the way
//! the web client's pin panel lists them. Enter jumps to one in the chat.

use crate::app::{App, PinsState};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

/// Rows one pin takes: who and when, what it says, a blank.
const ROWS_PER_PIN: usize = 3;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(view) = app.pins.as_ref() else {
        return;
    };
    frame.render_widget(Clear, area);
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(6),
            Constraint::Length(1),
        ])
        .split(area);
    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(32),
            Constraint::Length(2),
        ])
        .split(outer[1]);
    let popup = mid[1];
    frame.render_widget(Clear, popup);
    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(popup);
    let content = body[0];

    let accent = Style::default()
        .fg(crate::ui::theme::accent())
        .add_modifier(Modifier::BOLD);
    let text = Style::default().fg(crate::ui::theme::text());
    let dim = crate::ui::theme::dim_style();
    let muted = crate::ui::theme::muted_style();

    let where_ = app
        .channel_by_id(&view.channel_id)
        .map(|c| crate::ui::sidebar::channel_name(app, c))
        .unwrap_or_default();
    let title = if where_.is_empty() {
        " Pinned ".to_string()
    } else {
        format!(" Pinned in {where_} ")
    };
    let block = Block::default()
        .title(Line::from(Span::styled(title, accent)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(crate::ui::theme::accent_dim()));

    let (lines, scroll): (Vec<Line>, u16) = match &view.state {
        PinsState::Loading => (vec![Line::from(Span::styled("  Loading…", muted))], 0),
        PinsState::Failed(message) => (
            vec![Line::from(Span::styled(format!("  {message}"), muted))],
            0,
        ),
        PinsState::Ready(items) if items.is_empty() => (
            vec![Line::from(Span::styled(
                "  Nothing is pinned in this channel.",
                muted,
            ))],
            0,
        ),
        PinsState::Ready(items) => {
            let mut lines = Vec::with_capacity(items.len() * ROWS_PER_PIN);
            for (index, pin) in items.iter().enumerate() {
                let selected = index == view.selected;
                let guild_id = app.guild_id_for_channel(&pin.message.channel_id);
                let author = app.shown_name_for_user(guild_id.as_deref(), &pin.message.author);
                let when = crate::ui::message_pane::format_timestamp(
                    &pin.message.timestamp,
                    app.ui_settings.clock_12h,
                );
                lines.push(Line::from(vec![
                    Span::styled(if selected { " ▸ " } else { "   " }, accent),
                    Span::styled(
                        author,
                        if selected {
                            text.add_modifier(Modifier::BOLD)
                        } else {
                            text
                        },
                    ),
                    Span::styled("  ", text),
                    Span::styled(when, if selected { accent } else { dim }),
                ]));
                let mut row = vec![Span::styled("     ", text)];
                row.extend(crate::ui::pings_overlay::preview_spans(
                    app,
                    &pin.message,
                    if selected { text } else { muted },
                ));
                lines.push(Line::from(row));
                lines.push(Line::from(""));
            }
            let inner = content.height.saturating_sub(2) as usize;
            let bottom = (view.selected + 1) * ROWS_PER_PIN;
            (lines, bottom.saturating_sub(inner) as u16)
        }
    };

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .scroll((scroll, 0))
            .alignment(Alignment::Left),
        content,
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "↑/↓ move  ·  Enter go to message  ·  x unpin  ·  R reload  ·  Esc close",
            muted,
        )))
        .alignment(Alignment::Center),
        body[1],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::{
        CHANNEL_DM, ChannelPinResponse, ChannelResponse, MessageResponse, UserPartialResponse,
    };
    use crate::app::ServerSelection;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn drawn(app: &App, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| render(f, f.area(), app)).unwrap();
        let buf = t.backend().buffer().clone();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn app() -> App {
        let mut app = App::new(
            Default::default(),
            Default::default(),
            None,
            Vec::new(),
            Vec::new(),
            ServerSelection::DirectMessages,
            None,
            Default::default(),
        );
        app.private_channels.push(ChannelResponse {
            id: "dm1".to_string(),
            kind: CHANNEL_DM,
            recipients: vec![UserPartialResponse {
                id: "u2".to_string(),
                username: "ada".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        });
        app.open_pins("dm1".to_string());
        app
    }

    fn pin(id: &str, content: &str) -> ChannelPinResponse {
        ChannelPinResponse {
            message: MessageResponse {
                id: id.to_string(),
                channel_id: "dm1".to_string(),
                author: UserPartialResponse {
                    id: "u2".to_string(),
                    username: "ada".to_string(),
                    ..Default::default()
                },
                content: content.to_string(),
                timestamp: "2026-09-10T10:00:00.000Z".to_string(),
                pinned: true,
                ..Default::default()
            },
            pinned_at: "2026-09-10T11:00:00.000Z".to_string(),
        }
    }

    #[test]
    fn the_list_shows_each_pin_and_marks_the_cursor() {
        let mut app = app();
        assert!(drawn(&app, 80, 20).contains("Loading…"));
        app.set_pins_loaded("dm1", vec![pin("2", "read me"), pin("1", "and me")]);
        let s = drawn(&app, 80, 20);
        assert!(s.contains("Pinned"), "{s}");
        assert!(s.contains("read me"), "{s}");
        assert!(s.contains("and me"), "{s}");
        assert_eq!(s.lines().filter(|l| l.contains("▸")).count(), 1, "{s}");
        app.pins_move(1);
        assert_eq!(app.pins_selected().map(|m| m.id), Some("1".to_string()));
    }

    #[test]
    fn an_empty_channel_and_a_failure_say_so() {
        let mut app = app();
        app.set_pins_loaded("dm1", Vec::new());
        assert!(drawn(&app, 80, 12).contains("Nothing is pinned in this channel."));
        app.set_pins_failed("dm1", "Failed to load pins: 403".to_string());
        assert!(drawn(&app, 80, 12).contains("Failed to load pins: 403"));
    }
}
