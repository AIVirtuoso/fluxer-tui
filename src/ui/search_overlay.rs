//! Message search: a query line at the top, the scope beside it, and the
//! hits below. Typing goes to the query; once an answer is in, moving
//! goes to the hits and Enter jumps to one in the chat.

use crate::app::{App, SearchState};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

/// Rows one hit takes: where and when, who and what, a blank.
const ROWS_PER_HIT: usize = 3;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(view) = app.search.as_ref() else {
        return;
    };
    frame.render_widget(Clear, area);
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(8),
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
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(popup);

    let accent = Style::default()
        .fg(crate::ui::theme::accent())
        .add_modifier(Modifier::BOLD);
    let text = Style::default().fg(crate::ui::theme::text());
    let dim = crate::ui::theme::dim_style();
    let muted = crate::ui::theme::muted_style();

    // the query line, with the scope in its title so it costs no row
    let query_block = Block::default()
        .title(Line::from(vec![
            Span::styled(" Search ", accent),
            Span::styled(
                format!("\u{2039} {} \u{203A} ", view.scope.label()),
                if view.editing { accent } else { muted },
            ),
        ]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(crate::ui::theme::accent_dim()));

    let mut query_spans = vec![Span::styled(view.query.clone(), text)];
    if view.editing {
        query_spans.push(Span::styled("\u{2588}", accent));
    }
    if view.query.is_empty() && !view.editing {
        query_spans = vec![Span::styled(
            "words, \"a phrase\", from:name, has:image, pinned:true",
            muted,
        )];
    }
    frame.render_widget(
        Paragraph::new(Line::from(query_spans)).block(query_block),
        body[0],
    );

    let content = body[1];
    let hits = app.search_results();
    let (page, pages) = app.search_pages();

    let results_title = match &view.state {
        SearchState::Ready { total, .. } if *total > 0 => {
            if pages > 1 {
                format!(" {total} found  ·  page {page} of {pages} ")
            } else {
                format!(" {total} found ")
            }
        }
        _ => " Results ".to_string(),
    };
    let block = Block::default()
        .title(Line::from(Span::styled(results_title, accent)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(crate::ui::theme::accent_dim()));

    let (lines, scroll): (Vec<Line>, u16) = match &view.state {
        SearchState::Idle => (
            vec![
                Line::from(Span::styled("  Type, then Enter.", muted)),
                Line::from(""),
                Line::from(Span::styled(
                    "  \"in quotes\" has to appear together.",
                    muted,
                )),
                Line::from(Span::styled(
                    "  from:someone  ·  has:image, sound, video, file, embed  ·  pinned:true",
                    muted,
                )),
            ],
            0,
        ),
        SearchState::Running => (vec![Line::from(Span::styled("  Searching…", muted))], 0),
        SearchState::Indexing => (
            vec![
                Line::from(Span::styled(
                    "  The server is still indexing a channel in scope.",
                    muted,
                )),
                Line::from(Span::styled("  Enter tries again.", muted)),
            ],
            0,
        ),
        SearchState::Failed(message) => (
            vec![Line::from(Span::styled(format!("  {message}"), muted))],
            0,
        ),
        SearchState::Ready { .. } if hits.is_empty() => (
            vec![Line::from(Span::styled("  Nothing matched.", muted))],
            0,
        ),
        SearchState::Ready { .. } => {
            let mut lines = Vec::with_capacity(hits.len() * ROWS_PER_HIT);
            for (index, message) in hits.iter().enumerate() {
                let selected = index == view.selected && !view.editing;
                let (guild, channel) = app.search_hit_location(&message.channel_id);
                let place = match guild {
                    Some(guild) => format!("#{channel} · {guild}"),
                    None => channel,
                };
                let when = crate::ui::message_pane::format_timestamp(
                    &message.timestamp,
                    app.ui_settings.clock_12h,
                );
                lines.push(Line::from(vec![
                    Span::styled(if selected { " \u{25B8} " } else { "   " }, accent),
                    Span::styled(when, if selected { accent } else { dim }),
                    Span::styled("  ", text),
                    Span::styled(place, if selected { accent } else { dim }),
                ]));
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
                lines.push(Line::from(row));
                lines.push(Line::from(""));
            }
            let inner = content.height.saturating_sub(2) as usize;
            let bottom = (view.selected + 1) * ROWS_PER_HIT;
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

    let footer = if view.editing {
        "type it  ·  Enter search  ·  \u{2190}/\u{2192} scope  ·  \u{2193} results  ·  Esc close"
    } else if pages > 1 {
        "\u{2191}/\u{2193} move  ·  Enter go to it  ·  n/p page  ·  / edit  ·  Esc close"
    } else {
        "\u{2191}/\u{2193} move  ·  Enter go to it  ·  / edit the query  ·  Esc close"
    };
    crate::ui::footer::render(frame, body[2], app, footer);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::{CHANNEL_DM, ChannelResponse, MessageResponse, UserPartialResponse};
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
        app.selected_channel_id = Some("dm1".to_string());
        app.open_search();
        app
    }

    fn hit(id: &str, content: &str) -> MessageResponse {
        MessageResponse {
            id: id.to_string(),
            channel_id: "dm1".to_string(),
            author: UserPartialResponse {
                id: "u2".to_string(),
                username: "ada".to_string(),
                ..Default::default()
            },
            content: content.to_string(),
            timestamp: "2026-09-10T10:00:00.000Z".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn it_opens_on_the_narrowest_scope_and_says_what_a_query_can_hold() {
        let app = app();
        let s = drawn(&app, 80, 22);
        assert!(s.contains("Search"), "{s}");
        assert!(s.contains("this channel"), "{s}");
        assert!(s.contains("from:someone"), "{s}");
    }

    #[test]
    fn results_are_listed_with_where_they_came_from() {
        let mut app = app();
        app.set_search_running(1);
        assert!(drawn(&app, 80, 22).contains("Searching…"));
        app.set_search_results(vec![hit("1", "found me")], Vec::new(), 1, 1, 25);
        let s = drawn(&app, 80, 22);
        assert!(s.contains("1 found"), "{s}");
        assert!(s.contains("ada: found me"), "{s}");
        assert_eq!(s.lines().filter(|l| l.contains("▸")).count(), 1, "{s}");
    }

    #[test]
    fn several_pages_are_counted_and_offer_the_paging_keys() {
        let mut app = app();
        app.set_search_results(vec![hit("1", "one")], Vec::new(), 60, 2, 25);
        let s = drawn(&app, 80, 22);
        assert!(s.contains("60 found"), "{s}");
        assert!(s.contains("page 2 of 3"), "{s}");
        assert!(s.contains("n/p page"), "{s}");
    }

    #[test]
    fn indexing_is_shown_as_an_answer_rather_than_a_failure() {
        let mut app = app();
        app.set_search_indexing();
        let s = drawn(&app, 80, 22);
        assert!(s.contains("still indexing"), "{s}");
        assert!(!s.contains("Nothing matched"), "{s}");
    }

    #[test]
    fn an_empty_answer_says_so() {
        let mut app = app();
        app.set_search_results(Vec::new(), Vec::new(), 0, 1, 25);
        assert!(drawn(&app, 80, 22).contains("Nothing matched."));
    }
}
