//! The member list, in a column to the right of the messages.
//!
//! The gateway sends this list in windows rather than whole (see
//! `App::apply_member_list_update`), grouped by hoisted role and then by
//! whether people are about, with a heading row carrying each group's
//! count. The pane draws the headings and the members in the order they
//! arrive in, which is the order the server sorted them into.

use crate::app::{App, MemberRow};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};

/// How wide the column is, when there is room for it at all.
pub const WIDTH: u16 = 22;

/// Whether the pane is worth showing at this terminal width: below this
/// the messages would be squeezed into nothing.
pub const MIN_TOTAL_WIDTH: u16 = 80;

/// The width the member column takes out of the body, or zero when it is
/// shut or there is no room.
pub fn width(app: &App, total_width: u16) -> u16 {
    if app.member_list.is_some() && total_width >= MIN_TOTAL_WIDTH {
        WIDTH
    } else {
        0
    }
}

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(list) = app.member_list.as_ref() else {
        return;
    };

    let accent = Style::default()
        .fg(crate::ui::theme::accent())
        .add_modifier(Modifier::BOLD);
    let text = Style::default().fg(crate::ui::theme::text());
    let muted = crate::ui::theme::muted_style();

    let title = if list.member_count > 0 {
        format!(" {} of {} ", list.online_count, list.member_count)
    } else {
        " Members ".to_string()
    };

    let block = Block::default()
        .title(Line::from(Span::styled(title, accent)))
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(crate::ui::theme::accent_dim()));

    let inner_w = area.width.saturating_sub(2).max(1) as usize;
    let rows = app.member_list_rows();

    let lines: Vec<Line> = if rows.is_empty() {
        let what = if list.loaded {
            "Nobody here."
        } else {
            "Loading…"
        };
        vec![Line::from(Span::styled(format!(" {what}"), muted))]
    } else {
        rows.iter()
            .map(|row| match row {
                MemberRow::Heading { label, count } => {
                    let name = app.member_group_label(&list.guild_id, label).to_uppercase();
                    let label = truncate(&format!("{name} — {count}"), inner_w);
                    Line::from(Span::styled(
                        format!(" {label}"),
                        muted.add_modifier(Modifier::BOLD),
                    ))
                }
                MemberRow::Member { user_id } => {
                    let status = app.presence_status(user_id);
                    let member = list.members.get(user_id);
                    let name = match member {
                        Some(member) => app.shown_name_for_user(Some(&list.guild_id), &member.user),
                        None => app
                            .user_cache
                            .get(user_id)
                            .map(crate::app::display_name)
                            .unwrap_or_else(|| "unknown".to_string()),
                    };
                    let colour = if *user_id == app.me.id {
                        crate::ui::theme::self_username_color()
                    } else {
                        app.member_name_color(Some(&list.guild_id), user_id, false)
                    };
                    // an offline member is dimmed the way the web client
                    // fades them, so the people about stand out
                    let name_style = if status.is_offline() {
                        Style::default().fg(colour).add_modifier(Modifier::DIM)
                    } else {
                        Style::default().fg(colour)
                    };
                    Line::from(vec![
                        Span::styled(" ", text),
                        crate::ui::presence::dot_prefix(status),
                        Span::styled(truncate(&name, inner_w.saturating_sub(2)), name_style),
                    ])
                }
            })
            .collect()
    };

    let visible = area.height.saturating_sub(1) as usize;
    let max_scroll = lines.len().saturating_sub(visible) as u16;
    let scroll = list.scroll.min(max_scroll);

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .scroll((scroll, 0)),
        area,
    );
}

fn truncate(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    let keep = width.saturating_sub(1);
    text.chars().take(keep).collect::<String>() + "\u{2026}"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::{
        CHANNEL_GUILD_TEXT, ChannelResponse, GuildMemberListUpdateEvent, GuildMemberResponse,
        GuildResponse, GuildRoleResponse, MemberListGroup, MemberListItem, MemberListMember,
        MemberListOp, PresenceRecord, UserPartialResponse,
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
        app.guilds.push(GuildResponse {
            id: "g1".to_string(),
            name: "Guild".to_string(),
            ..Default::default()
        });
        app.guild_channels.insert(
            "g1".to_string(),
            vec![ChannelResponse {
                id: "c1".to_string(),
                kind: CHANNEL_GUILD_TEXT,
                guild_id: Some("g1".to_string()),
                name: "general".to_string(),
                ..Default::default()
            }],
        );
        app.guild_roles.insert(
            "g1".to_string(),
            vec![GuildRoleResponse {
                id: "r1".to_string(),
                name: "Maintainers".to_string(),
                ..Default::default()
            }],
        );
        app.selected_server = ServerSelection::Guild("g1".to_string());
        app.selected_channel_id = Some("c1".to_string());
        app
    }

    fn member_item(user_id: &str, name: &str, status: &str) -> MemberListItem {
        MemberListItem {
            group: None,
            member: Some(MemberListMember {
                member: GuildMemberResponse {
                    user: UserPartialResponse {
                        id: user_id.to_string(),
                        username: name.to_string(),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                presence: Some(PresenceRecord {
                    status: Some(status.to_string()),
                    ..Default::default()
                }),
            }),
        }
    }

    fn group_item(id: &str, count: u32) -> MemberListItem {
        MemberListItem {
            group: Some(MemberListGroup {
                id: id.to_string(),
                count,
            }),
            member: None,
        }
    }

    fn update(items: Vec<MemberListItem>, start: u32, end: u32) -> GuildMemberListUpdateEvent {
        GuildMemberListUpdateEvent {
            guild_id: "g1".to_string(),
            id: "c1".to_string(),
            channel_id: Some("c1".to_string()),
            member_count: 3,
            online_count: 2,
            ops: vec![MemberListOp {
                op: "SYNC".to_string(),
                range: vec![start, end],
                items,
            }],
        }
    }

    #[test]
    fn headings_are_named_from_the_roles_and_members_carry_their_presence() {
        let mut app = app();
        app.toggle_member_list().expect("opens in a guild channel");
        assert!(drawn(&app, 24, 10).contains("Loading…"));
        app.apply_member_list_update(update(
            vec![
                group_item("r1", 1),
                member_item("u1", "ada", "online"),
                group_item("offline", 1),
                member_item("u2", "bob", "offline"),
            ],
            0,
            3,
        ));
        let s = drawn(&app, 24, 10);
        // a hoisted role group is named by the role, not its id
        assert!(s.contains("MAINTAINERS"), "{s}");
        assert!(s.contains("OFFLINE"), "{s}");
        assert!(s.contains("ada"), "{s}");
        assert!(s.contains("bob"), "{s}");
        // the counts go in the title
        assert!(s.contains("2 of 3"), "{s}");
        // and the presences the list carried are now the client's
        assert_eq!(
            app.presence_status("u1"),
            crate::api::types::PresenceStatus::Online
        );
        assert_eq!(
            app.presence_status("u2"),
            crate::api::types::PresenceStatus::Offline
        );
    }

    #[test]
    fn a_sync_replaces_the_rows_of_its_range_and_nothing_else() {
        let mut app = app();
        app.toggle_member_list();
        app.apply_member_list_update(update(
            vec![
                member_item("u1", "ada", "online"),
                member_item("u2", "bob", "online"),
            ],
            0,
            1,
        ));
        assert!(drawn(&app, 24, 10).contains("ada"));
        // the same range comes back with one row: the other is gone
        app.apply_member_list_update(update(vec![member_item("u3", "cal", "online")], 0, 1));
        let s = drawn(&app, 24, 10);
        assert!(s.contains("cal"), "{s}");
        assert!(!s.contains("ada"), "{s}");
        assert!(!s.contains("bob"), "{s}");
    }

    #[test]
    fn an_update_for_another_channel_is_ignored() {
        let mut app = app();
        app.toggle_member_list();
        let mut event = update(vec![member_item("u1", "ada", "online")], 0, 0);
        event.channel_id = Some("c2".to_string());
        app.apply_member_list_update(event);
        assert!(drawn(&app, 24, 10).contains("Loading…"));
    }

    #[test]
    fn the_column_is_dropped_on_a_narrow_terminal_rather_than_squeezing_the_messages() {
        let mut app = app();
        app.toggle_member_list();
        assert_eq!(width(&app, 120), WIDTH);
        assert_eq!(width(&app, MIN_TOTAL_WIDTH), WIDTH);
        assert_eq!(width(&app, MIN_TOTAL_WIDTH - 1), 0);
        app.close_member_list();
        assert_eq!(width(&app, 120), 0);
    }
}
