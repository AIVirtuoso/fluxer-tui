//! Friends, the requests in both directions, and the accounts the reader
//! has blocked — the four groups the server sorts relationships into, as
//! four tabs of one list.

use crate::app::{App, FriendsState, FriendsTab};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(view) = app.friends.as_ref() else {
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

    // the tabs go in the title, which costs no row of the list
    let mut title = vec![Span::styled(" ", accent)];
    for tab in FriendsTab::ALL {
        let count = app.relationships_in(tab).len();
        let selected = tab == view.tab;
        title.push(Span::styled(
            format!("{} {}", tab.label(), count),
            if selected { accent } else { muted },
        ));
        title.push(Span::styled("  ", dim));
    }

    let block = Block::default()
        .title(Line::from(title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(crate::ui::theme::accent_dim()));

    let rows = app.relationships_in(view.tab);
    let inner_h = content.height.saturating_sub(2) as usize;

    let (lines, scroll): (Vec<Line>, u16) = match &view.state {
        FriendsState::Loading => (vec![Line::from(Span::styled("  Loading…", muted))], 0),
        FriendsState::Failed(message) => (
            vec![Line::from(Span::styled(format!("  {message}"), muted))],
            0,
        ),
        FriendsState::Ready if rows.is_empty() => (
            vec![Line::from(Span::styled(
                format!("  {}", empty_label(view.tab)),
                muted,
            ))],
            0,
        ),
        FriendsState::Ready => {
            let lines: Vec<Line> = rows
                .iter()
                .enumerate()
                .map(|(index, relationship)| {
                    let selected = index == view.selected;
                    let status = app.presence_status(&relationship.user.id);
                    let name = app
                        .relationship_nickname(&relationship.user.id)
                        .map(str::to_string)
                        .unwrap_or_else(|| crate::app::display_name(&relationship.user));
                    let tag = format!(
                        "{}#{}",
                        relationship.user.username, relationship.user.discriminator
                    );
                    let mut spans = vec![
                        Span::styled(if selected { " \u{25B8} " } else { "   " }, accent),
                        crate::ui::presence::dot_prefix(status),
                        Span::styled(
                            name,
                            if selected {
                                text.add_modifier(Modifier::BOLD)
                            } else {
                                text
                            },
                        ),
                        Span::styled(format!("  {tag}"), dim),
                    ];
                    if app.relationship_nickname(&relationship.user.id).is_some() {
                        spans.push(Span::styled("  (your name for them)", muted));
                    }
                    Line::from(spans)
                })
                .collect();
            let scroll = (view.selected + 1).saturating_sub(inner_h) as u16;
            (lines, scroll)
        }
    };

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .scroll((scroll, 0))
            .alignment(Alignment::Left),
        content,
    );

    let footer = match &view.input {
        Some(input) => format!(
            "{}: {}\u{2588}  ·  Enter save  ·  Esc cancel",
            input.prompt(),
            input.text()
        ),
        None => footer_for(view.tab).to_string(),
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(footer, muted))).alignment(Alignment::Center),
        body[1],
    );
}

fn empty_label(tab: FriendsTab) -> &'static str {
    match tab {
        FriendsTab::Friends => "No friends yet (+ adds one by their tag).",
        FriendsTab::Incoming => "Nobody has asked to be friends.",
        FriendsTab::Outgoing => "You have not asked anybody.",
        FriendsTab::Blocked => "You have blocked nobody.",
    }
}

fn footer_for(tab: FriendsTab) -> &'static str {
    match tab {
        FriendsTab::Friends => {
            "\u{2190}/\u{2192} group · Enter message · n name · + add · x unfriend · B block · Esc"
        }
        FriendsTab::Incoming => {
            "\u{2190}/\u{2192} group  ·  a accept  ·  x turn down  ·  B block  ·  + add  ·  Esc close"
        }
        FriendsTab::Outgoing => {
            "\u{2190}/\u{2192} group  ·  x take it back  ·  + add  ·  Esc close"
        }
        FriendsTab::Blocked => "\u{2190}/\u{2192} group  ·  x unblock  ·  Esc close",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::{
        RELATIONSHIP_BLOCKED, RELATIONSHIP_FRIEND, RELATIONSHIP_INCOMING_REQUEST,
        RelationshipResponse, UserPartialResponse,
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

    fn relationship(id: &str, name: &str, kind: i32) -> RelationshipResponse {
        RelationshipResponse {
            id: format!("r{id}"),
            relationship_type: kind,
            user: UserPartialResponse {
                id: id.to_string(),
                username: name.to_string(),
                discriminator: "0001".to_string(),
                ..Default::default()
            },
            since: None,
            nickname: None,
        }
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
        app.open_friends();
        app
    }

    #[test]
    fn the_four_groups_are_counted_in_the_title_and_only_one_is_listed() {
        let mut app = app();
        assert!(drawn(&app, 70, 14).contains("Loading…"));
        app.set_relationships(vec![
            relationship("u1", "ada", RELATIONSHIP_FRIEND),
            relationship("u2", "bob", RELATIONSHIP_INCOMING_REQUEST),
            relationship("u3", "cal", RELATIONSHIP_BLOCKED),
        ]);
        let s = drawn(&app, 70, 14);
        assert!(s.contains("Friends 1"), "{s}");
        assert!(s.contains("Wanting 1"), "{s}");
        assert!(s.contains("Blocked 1"), "{s}");
        assert!(s.contains("Asked 0"), "{s}");
        // only the group under the cursor is listed
        assert!(s.contains("ada"), "{s}");
        assert!(!s.contains("bob"), "{s}");
        assert!(!s.contains("cal"), "{s}");
        // and the tag is shown, since that is how you are added
        assert!(s.contains("ada#0001"), "{s}");
    }

    #[test]
    fn switching_group_lists_the_others_and_offers_their_keys() {
        let mut app = app();
        app.set_relationships(vec![
            relationship("u1", "ada", RELATIONSHIP_FRIEND),
            relationship("u2", "bob", RELATIONSHIP_INCOMING_REQUEST),
        ]);
        app.friends_switch_tab(true);
        let s = drawn(&app, 70, 14);
        assert!(s.contains("bob"), "{s}");
        assert!(s.contains("a accept"), "{s}");
        // and back round through all four
        for _ in 0..3 {
            app.friends_switch_tab(true);
        }
        assert!(drawn(&app, 70, 14).contains("ada"));
    }

    #[test]
    fn an_empty_group_says_what_would_fill_it() {
        let mut app = app();
        app.set_relationships(Vec::new());
        assert!(drawn(&app, 70, 12).contains("No friends yet"));
        app.friends_switch_tab(false);
        assert!(drawn(&app, 70, 12).contains("You have blocked nobody."));
    }

    #[test]
    fn adding_by_tag_takes_over_the_footer() {
        let mut app = app();
        app.set_relationships(Vec::new());
        if let Some(view) = app.friends.as_mut() {
            view.input = Some(crate::app::FriendsInput::AddTag("ada#0001".to_string()));
        }
        assert!(drawn(&app, 70, 12).contains("Add by tag: ada#0001"));
    }
}
