//! Who reacted to a message with one emoji. Left and Right walk the
//! message's other reactions without closing, the way the web client's
//! reactions sheet has a tab per emoji.

use crate::app::{App, ReactionUsersState};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(view) = app.reaction_users.as_ref() else {
        return;
    };
    frame.render_widget(Clear, area);

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(15),
            Constraint::Percentage(70),
            Constraint::Percentage(15),
        ])
        .split(area);
    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(60),
            Constraint::Percentage(20),
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
    let muted = crate::ui::theme::muted_style();

    // how many reactions the message carries, so the title can say which
    // of them is showing
    let total = app
        .message_by_id(&view.channel_id, &view.message_id)
        .map(|m| m.reactions.len())
        .unwrap_or(0);
    let title = if total > 1 {
        format!(
            " Reacted with {}  ({} of {}) ",
            view.emoji_label,
            view.reaction_index + 1,
            total
        )
    } else {
        format!(" Reacted with {} ", view.emoji_label)
    };

    let block = Block::default()
        .title(Line::from(Span::styled(title, accent)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(crate::ui::theme::accent_dim()));

    let guild_id = app.guild_id_for_channel(&view.channel_id);
    let lines: Vec<Line> = match &view.state {
        ReactionUsersState::Loading => vec![Line::from(Span::styled("  Loading…", muted))],
        ReactionUsersState::Failed(message) => {
            vec![Line::from(Span::styled(format!("  {message}"), muted))]
        }
        ReactionUsersState::Ready(users) if users.is_empty() => {
            vec![Line::from(Span::styled("  Nobody, any more.", muted))]
        }
        ReactionUsersState::Ready(users) => users
            .iter()
            .map(|user| {
                let name = app.shown_name_for_user(guild_id.as_deref(), user);
                let colour = if user.id == app.me.id {
                    crate::ui::theme::self_username_color()
                } else {
                    crate::ui::theme::username_color(&user.id)
                };
                Line::from(vec![
                    Span::styled("  ", text),
                    Span::styled(name, Style::default().fg(colour)),
                ])
            })
            .collect(),
    };

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .scroll((view.scroll, 0))
            .alignment(Alignment::Left),
        content,
    );

    let footer = if total > 1 {
        "↑/↓ scroll  ·  ←/→ another reaction  ·  Esc close"
    } else {
        "↑/↓ scroll  ·  Esc close"
    };
    crate::ui::footer::render(frame, body[1], app, footer);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::{
        CHANNEL_DM, ChannelResponse, MessageReactionResponse, MessageResponse,
        ReactionEmojiResponse, UserPartialResponse,
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

    fn reaction(name: &str, id: Option<&str>) -> MessageReactionResponse {
        MessageReactionResponse {
            emoji: ReactionEmojiResponse {
                id: id.map(str::to_string),
                name: name.to_string(),
                animated: false,
            },
            count: 2,
            me: false,
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
        app.private_channels.push(ChannelResponse {
            id: "dm1".to_string(),
            kind: CHANNEL_DM,
            ..Default::default()
        });
        app.selected_channel_id = Some("dm1".to_string());
        app.set_channel_messages(
            "dm1",
            vec![MessageResponse {
                id: "m1".to_string(),
                channel_id: "dm1".to_string(),
                content: "hi".to_string(),
                timestamp: "2026-09-10T10:00:00.000Z".to_string(),
                reactions: vec![reaction("👍", None), reaction("party", Some("99"))],
                ..Default::default()
            }],
        );
        app
    }

    #[test]
    fn it_names_the_emoji_and_lists_who_reacted() {
        let mut app = app();
        let (_, emoji) = app.open_reaction_users("m1", 0).expect("open");
        assert_eq!(emoji, "👍");
        assert!(drawn(&app, 60, 14).contains("Loading…"));
        app.set_reaction_users_loaded(
            "👍",
            vec![UserPartialResponse {
                id: "u2".to_string(),
                username: "ada".to_string(),
                ..Default::default()
            }],
        );
        let s = drawn(&app, 60, 14);
        assert!(s.contains("Reacted with 👍"), "{s}");
        assert!(s.contains("1 of 2"), "{s}");
        assert!(s.contains("ada"), "{s}");
    }

    #[test]
    fn right_walks_to_the_next_reaction_and_asks_for_it() {
        let mut app = app();
        app.open_reaction_users("m1", 0).expect("open");
        let (_, _, emoji) = app.reaction_users_step(1).expect("step");
        // a custom emoji goes to the API as name:id and shows as :name:
        assert_eq!(emoji, "party:99");
        let s = drawn(&app, 60, 14);
        assert!(s.contains("Reacted with :party:"), "{s}");
        assert!(s.contains("2 of 2"), "{s}");
        // and it wraps back round
        let (_, _, emoji) = app.reaction_users_step(1).expect("wrap");
        assert_eq!(emoji, "👍");
    }
}
