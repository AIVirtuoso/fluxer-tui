//! The messages the user has bookmarked, newest first, from every
//! community and direct message at once — the web client's saved
//! messages. An entry whose message the server can no longer hand over
//! says so rather than vanishing, so the bookmark can still be dropped.

use crate::app::{App, SavedState};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

/// Rows one bookmark takes: where and when, who and what, a blank.
const ROWS_PER_ENTRY: usize = 3;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(view) = app.saved.as_ref() else {
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

    let block = Block::default()
        .title(Line::from(Span::styled(" Bookmarks ", accent)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(crate::ui::theme::accent_dim()));

    let (lines, scroll): (Vec<Line>, u16) = match &view.state {
        SavedState::Loading => (vec![Line::from(Span::styled("  Loading…", muted))], 0),
        SavedState::Failed(message) => (
            vec![Line::from(Span::styled(format!("  {message}"), muted))],
            0,
        ),
        SavedState::Ready(entries) if entries.is_empty() => (
            vec![Line::from(Span::styled(
                "  You have not bookmarked anything yet (b on a message).",
                muted,
            ))],
            0,
        ),
        SavedState::Ready(entries) => {
            let mut lines = Vec::with_capacity(entries.len() * ROWS_PER_ENTRY);
            for (index, entry) in entries.iter().enumerate() {
                let selected = index == view.selected;
                let (guild, channel) = app.channel_location(&entry.channel_id);
                let place = match guild {
                    Some(guild) => format!("#{channel} · {guild}"),
                    None => channel,
                };
                let when = entry
                    .message
                    .as_ref()
                    .map(|m| {
                        crate::ui::message_pane::format_timestamp(
                            &m.timestamp,
                            app.ui_settings.clock_12h,
                        )
                    })
                    .unwrap_or_default();
                lines.push(Line::from(vec![
                    Span::styled(if selected { " ▸ " } else { "   " }, accent),
                    Span::styled(when, if selected { accent } else { dim }),
                    Span::styled("  ", text),
                    Span::styled(place, if selected { accent } else { dim }),
                ]));
                let row = match &entry.message {
                    Some(message) => {
                        let guild_id = app.guild_id_for_channel(&message.channel_id);
                        let author = app.shown_name_for_user(guild_id.as_deref(), &message.author);
                        let mut row = vec![
                            Span::styled("     ", text),
                            Span::styled(
                                format!("{author}: "),
                                if selected {
                                    text.add_modifier(Modifier::BOLD)
                                } else {
                                    text
                                },
                            ),
                        ];
                        row.extend(crate::ui::pings_overlay::preview_spans(
                            app,
                            message,
                            if selected { text } else { muted },
                        ));
                        row
                    }
                    None => vec![
                        Span::styled("     ", text),
                        Span::styled(
                            unavailable_label(&entry.status),
                            muted.add_modifier(Modifier::ITALIC),
                        ),
                    ],
                };
                lines.push(Line::from(row));
                lines.push(Line::from(""));
            }
            let inner = content.height.saturating_sub(2) as usize;
            let bottom = (view.selected + 1) * ROWS_PER_ENTRY;
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
            "↑/↓ move  ·  Enter go to message  ·  x remove the bookmark  ·  R reload  ·  Esc close",
            muted,
        )))
        .alignment(Alignment::Center),
        body[1],
    );
}

/// What to say about a bookmark whose message did not come back.
fn unavailable_label(status: &str) -> &'static str {
    match status {
        "deleted" => "(the message was deleted)",
        "unavailable" => "(the message is out of reach now)",
        _ => "(the message could not be loaded)",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::{
        CHANNEL_DM, ChannelResponse, MessageResponse, SavedMessageEntryResponse,
        UserPartialResponse,
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
        app.open_saved();
        app
    }

    fn entry(id: &str, content: Option<&str>) -> SavedMessageEntryResponse {
        SavedMessageEntryResponse {
            id: format!("s{id}"),
            channel_id: "dm1".to_string(),
            message_id: id.to_string(),
            status: if content.is_some() {
                "available".to_string()
            } else {
                "deleted".to_string()
            },
            message: content.map(|c| MessageResponse {
                id: id.to_string(),
                channel_id: "dm1".to_string(),
                author: UserPartialResponse {
                    id: "u2".to_string(),
                    username: "ada".to_string(),
                    ..Default::default()
                },
                content: c.to_string(),
                timestamp: "2026-09-10T10:00:00.000Z".to_string(),
                ..Default::default()
            }),
        }
    }

    #[test]
    fn the_list_shows_bookmarks_and_says_when_one_is_gone() {
        let mut app = app();
        assert!(drawn(&app, 80, 20).contains("Loading…"));
        app.set_saved_loaded(vec![entry("2", Some("keep this")), entry("1", None)]);
        let s = drawn(&app, 80, 20);
        assert!(s.contains("Bookmarks"), "{s}");
        assert!(s.contains("ada: keep this"), "{s}");
        assert!(s.contains("(the message was deleted)"), "{s}");
        assert_eq!(s.lines().filter(|l| l.contains("▸")).count(), 1, "{s}");
        // the ids are remembered, so a message can be shown as bookmarked
        assert!(app.is_bookmarked("2"));
        assert!(app.is_bookmarked("1"));
    }

    #[test]
    fn dropping_one_takes_it_off_the_open_list() {
        let mut app = app();
        app.set_saved_loaded(vec![entry("2", Some("keep this")), entry("1", None)]);
        app.forget_saved_message("2");
        let s = drawn(&app, 80, 20);
        assert!(!s.contains("keep this"), "{s}");
        assert!(!app.is_bookmarked("2"));
    }

    #[test]
    fn an_empty_list_says_how_to_fill_it() {
        let mut app = app();
        app.set_saved_loaded(Vec::new());
        assert!(drawn(&app, 80, 12).contains("You have not bookmarked anything yet"));
    }
}
