//! Starting a conversation, and looking after a group.
//!
//! One overlay in three modes: the people to start with, the handful of
//! things a group needs doing to it, and which of its people to take out.
//! They are all a list and a cursor, so they share the frame.

use crate::app::{App, ConversationMode};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(view) = app.conversation.as_ref() else {
        return;
    };
    frame.render_widget(Clear, area);
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(10),
            Constraint::Percentage(80),
            Constraint::Percentage(10),
        ])
        .split(area);
    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(10),
            Constraint::Percentage(80),
            Constraint::Percentage(10),
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

    let marked = app.conversation_marked().len();

    let (title, rows): (String, Vec<Line>) = match &view.mode {
        ConversationMode::People => {
            let title = if marked > 0 {
                format!(" New group  ·  {marked} picked ")
            } else {
                " New conversation ".to_string()
            };
            let matches = app.conversation_matches();
            let rows = if matches.is_empty() {
                vec![Line::from(Span::styled(
                    if view.filter.trim().is_empty() {
                        "  Nobody the client knows yet."
                    } else {
                        "  Nobody by that name."
                    },
                    muted,
                ))]
            } else {
                matches
                    .iter()
                    .enumerate()
                    .map(|(index, candidate)| {
                        let selected = index == view.selected;
                        let ticked = app.conversation_is_marked(&candidate.user.id);
                        Line::from(vec![
                            Span::styled(if selected { " \u{25B8} " } else { "   " }, accent),
                            Span::styled(
                                if ticked { "[x] " } else { "[ ] " },
                                if ticked { accent } else { dim },
                            ),
                            Span::styled(
                                crate::app::display_name(&candidate.user),
                                if selected {
                                    text.add_modifier(Modifier::BOLD)
                                } else {
                                    text
                                },
                            ),
                            Span::styled(
                                format!(
                                    "  {}#{}",
                                    candidate.user.username, candidate.user.discriminator
                                ),
                                dim,
                            ),
                            Span::styled(format!("   {}", candidate.note), muted),
                        ])
                    })
                    .collect()
            };
            (title, rows)
        }
        ConversationMode::Group { channel_id } => {
            let name = app
                .channel_by_id(channel_id)
                .map(|c| crate::ui::sidebar::channel_name(app, c))
                .unwrap_or_default();
            let rows = app
                .group_actions(channel_id)
                .iter()
                .enumerate()
                .map(|(index, action)| {
                    let selected = index == view.selected;
                    Line::from(vec![
                        Span::styled(if selected { " \u{25B8} " } else { "   " }, accent),
                        Span::styled(
                            action.label(),
                            if selected {
                                text.add_modifier(Modifier::BOLD)
                            } else {
                                text
                            },
                        ),
                    ])
                })
                .collect();
            (format!(" {name} "), rows)
        }
        ConversationMode::RemoveFrom { channel_id } => {
            let members = app.group_members(channel_id);
            let rows = if members.is_empty() {
                vec![Line::from(Span::styled("  Nobody else is in it.", muted))]
            } else {
                members
                    .iter()
                    .enumerate()
                    .map(|(index, user)| {
                        let selected = index == view.selected;
                        Line::from(vec![
                            Span::styled(if selected { " \u{25B8} " } else { "   " }, accent),
                            Span::styled(
                                crate::app::display_name(user),
                                if selected {
                                    text.add_modifier(Modifier::BOLD)
                                } else {
                                    text
                                },
                            ),
                        ])
                    })
                    .collect()
            };
            (" Take who out? ".to_string(), rows)
        }
    };

    let block = Block::default()
        .title(Line::from(Span::styled(title, accent)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(crate::ui::theme::accent_dim()));

    let inner = content.height.saturating_sub(2) as usize;
    let scroll = (view.selected + 1).saturating_sub(inner) as u16;

    frame.render_widget(
        Paragraph::new(Text::from(rows))
            .block(block)
            .scroll((scroll, 0))
            .alignment(Alignment::Left),
        content,
    );

    let footer = match (&view.input, &view.mode) {
        (Some(crate::app::ConversationInput::Rename { text, .. }), _) => {
            format!("Name: {text}\u{2588}  ·  Enter save  ·  empty clears it  ·  Esc cancel")
        }
        (None, ConversationMode::People) => {
            let action = if marked > 0 {
                format!("Enter make a group of {}", marked + 1)
            } else {
                "Enter open the conversation".to_string()
            };
            format!(
                "type to filter: {}\u{2588}  ·  Space pick  ·  {action}  ·  Esc close",
                view.filter
            )
        }
        (None, ConversationMode::Group { .. }) => {
            "\u{2191}/\u{2193} move  ·  Enter do it  ·  Esc close".to_string()
        }
        (None, ConversationMode::RemoveFrom { .. }) => {
            "\u{2191}/\u{2193} move  ·  Enter take them out  ·  Esc back".to_string()
        }
    };
    crate::ui::footer::render(frame, body[1], app, &footer);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::{CHANNEL_DM, CHANNEL_GROUP_DM, ChannelResponse, UserPartialResponse};
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

    fn user(id: &str, name: &str) -> UserPartialResponse {
        UserPartialResponse {
            id: id.to_string(),
            username: name.to_string(),
            discriminator: "0001".to_string(),
            ..Default::default()
        }
    }

    fn app() -> App {
        let mut me = crate::api::types::UserPrivateResponse::default();
        me.id = "me".to_string();
        let mut app = App::new(
            Default::default(),
            me,
            None,
            Vec::new(),
            vec![
                ChannelResponse {
                    id: "dm1".to_string(),
                    kind: CHANNEL_DM,
                    recipients: vec![user("u1", "ada")],
                    ..Default::default()
                },
                ChannelResponse {
                    id: "g1".to_string(),
                    kind: CHANNEL_GROUP_DM,
                    name: "the fork".to_string(),
                    recipients: vec![user("u2", "bob"), user("u3", "cal")],
                    ..Default::default()
                },
            ],
            ServerSelection::DirectMessages,
            None,
            Default::default(),
        );
        app.selected_channel_id = Some("g1".to_string());
        app
    }

    #[test]
    fn the_people_list_names_where_each_is_known_from_and_filters() {
        let mut app = app();
        app.open_new_conversation();
        let s = drawn(&app, 80, 20);
        assert!(s.contains("New conversation"), "{s}");
        assert!(s.contains("ada"), "{s}");
        assert!(s.contains("bob"), "{s}");
        assert!(s.contains("you talk already"), "{s}");
        assert!(s.contains("in a group with you"), "{s}");
        for c in "bo".chars() {
            app.conversation_filter_push(c);
        }
        let s = drawn(&app, 80, 20);
        assert!(s.contains("bob"), "{s}");
        assert!(!s.contains("ada"), "{s}");
        assert!(s.contains("type to filter: bo"), "{s}");
    }

    #[test]
    fn ticking_people_turns_it_into_a_group_of_the_right_size() {
        let mut app = app();
        app.open_new_conversation();
        assert!(drawn(&app, 80, 20).contains("Enter open the conversation"));
        assert_eq!(app.conversation_toggle_mark(), Some(1));
        app.conversation_move(1);
        assert_eq!(app.conversation_toggle_mark(), Some(2));
        let s = drawn(&app, 80, 20);
        // two picked plus the reader is a group of three
        assert!(s.contains("New group  ·  2 picked"), "{s}");
        assert!(s.contains("Enter make a group of 3"), "{s}");
        assert!(s.contains("[x]"), "{s}");
        // and ticking again unticks
        assert_eq!(app.conversation_toggle_mark(), Some(1));
    }

    #[test]
    fn the_group_menu_offers_what_a_group_needs_and_names_it() {
        let mut app = app();
        assert!(app.open_group_menu());
        let s = drawn(&app, 80, 20);
        assert!(s.contains("the fork"), "{s}");
        assert!(s.contains("Rename it"), "{s}");
        assert!(s.contains("Add somebody"), "{s}");
        assert!(s.contains("Take somebody out"), "{s}");
        assert!(s.contains("Leave it"), "{s}");
    }

    #[test]
    fn a_one_to_one_has_no_group_menu() {
        let mut app = app();
        app.selected_channel_id = Some("dm1".to_string());
        assert!(!app.open_group_menu());
        assert!(app.conversation.is_none());
    }

    #[test]
    fn taking_somebody_out_lists_only_the_others() {
        let mut app = app();
        app.open_group_menu();
        if let Some(view) = app.conversation.as_mut() {
            view.mode = ConversationMode::RemoveFrom {
                channel_id: "g1".to_string(),
            };
            view.selected = 0;
        }
        let s = drawn(&app, 80, 20);
        assert!(s.contains("Take who out?"), "{s}");
        assert!(s.contains("bob"), "{s}");
        assert!(s.contains("cal"), "{s}");
    }
}
