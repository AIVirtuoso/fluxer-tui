//! The profile popup (`p` on a selected message): what the web app's
//! profile card shows, from `GET /users/{id}/profile`.

use crate::api::types::{UserProfileResponse, user_flags};
use crate::app::{App, ProfileState, ProfileView};
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

/// The avatar block: square on a terminal whose cells are twice as tall as
/// wide.
const AVATAR_COLS: u16 = 8;
const AVATAR_ROWS: u16 = 4;

const FLUXER_EPOCH_MS: i64 = 1_420_070_400_000;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(view) = app.profile.as_ref() else {
        return;
    };
    let width = area.width.saturating_sub(4).clamp(28, 72);
    let avatar = app.avatars_enabled() && width >= AVATAR_COLS + 16;
    let lines = build_lines(app, view, width.saturating_sub(2) as usize, avatar);
    let paragraph = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false });
    // borders, the content, and the hint row: as tall as the content needs
    let rows = paragraph.line_count(width.saturating_sub(2)) as u16;
    let height = rows
        .saturating_add(3)
        .clamp(8, area.height.saturating_sub(2).max(8));
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);

    let accent = Style::default()
        .fg(crate::ui::theme::accent())
        .add_modifier(Modifier::BOLD);
    let block = Block::default()
        .title(Line::from(Span::styled(" Profile ", accent)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(crate::ui::theme::accent_dim()));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.height < 2 || inner.width < 8 {
        return;
    }
    let content = Rect {
        height: inner.height - 1,
        ..inner
    };
    let hint_area = Rect {
        y: inner.y + inner.height - 1,
        height: 1,
        ..inner
    };

    let avatar = avatar && content.height >= AVATAR_ROWS;
    let scroll = view.scroll.min(rows.saturating_sub(content.height));
    frame.render_widget(paragraph.scroll((scroll, 0)), content);

    // The avatar sits in the blank left margin of the header rows; the
    // media overlay draws the picture over these marker cells.
    if avatar && scroll == 0 {
        let member = app.profile_member();
        let slot = app.avatar_slot_sized(
            view.guild_id.as_deref(),
            &view.user,
            member.as_ref().and_then(|m| m.avatar.as_deref()),
            AVATAR_COLS,
            AVATAR_ROWS,
        );
        let k = app.register_media_slot(slot);
        let buf = frame.buffer_mut();
        for r in 0..AVATAR_ROWS {
            let style = crate::app::media_marker_style(k, r);
            for c in 0..AVATAR_COLS {
                buf[(content.x + c, content.y + r)]
                    .set_symbol("\u{2800}")
                    .set_style(style);
            }
        }
    }

    let hint = Paragraph::new(Line::from(Span::styled(
        "↑/↓ scroll  ·  p picture  ·  Esc / q close",
        crate::ui::theme::muted_style(),
    )))
    .alignment(Alignment::Center);
    frame.render_widget(hint, hint_area);
}

fn build_lines(app: &App, view: &ProfileView, width: usize, avatar: bool) -> Vec<Line<'static>> {
    let text = Style::default().fg(crate::ui::theme::text());
    let dim = crate::ui::theme::dim_style();
    let muted = crate::ui::theme::muted_style();
    let danger = Style::default().fg(crate::ui::theme::danger());
    let pill = crate::ui::theme::pill_style(crate::ui::theme::accent());

    let profile: Option<&UserProfileResponse> = match &view.state {
        ProfileState::Ready(p) => Some(p.as_ref()),
        _ => None,
    };
    let member = app.profile_member();
    let gid = view.guild_id.as_deref();
    let guild_name = gid.and_then(|g| {
        app.guilds
            .iter()
            .find(|x| x.id == g)
            .map(|x| x.name.clone())
    });
    let shown = profile.map(|p| p.shown_profile()).unwrap_or_default();
    let user = &view.user;
    let is_self = user.id == app.me.id;

    // Header: the first AVATAR_ROWS lines leave room for the picture.
    let margin: String = if avatar {
        " ".repeat(AVATAR_COLS as usize + 1)
    } else {
        " ".to_string()
    };
    let header_w = width.saturating_sub(margin.width()).max(8);
    let mut lines: Vec<Line<'static>> = Vec::new();

    let name_color = app.member_name_color(gid, &user.id, is_self);
    let nick = member
        .as_ref()
        .and_then(|m| m.nick.clone())
        .filter(|n| !n.trim().is_empty());
    let display = crate::app::display_name(user);
    let mut first: Vec<Span<'static>> = vec![Span::raw(margin.clone())];
    first.push(Span::styled(
        fit(nick.as_deref().unwrap_or(&display), header_w),
        Style::default().fg(name_color).add_modifier(Modifier::BOLD),
    ));
    for badge in badges(user, profile) {
        first.push(Span::raw(" "));
        first.push(Span::styled(format!(" {badge} "), pill));
    }
    lines.push(Line::from(first));

    let mut tag = format!("{}#{}", user.username, user.discriminator);
    if nick.is_some() && display != user.username {
        tag = format!("{display}  ·  {tag}");
    }
    lines.push(Line::from(vec![
        Span::raw(margin.clone()),
        Span::styled(fit(&tag, header_w), dim),
    ]));

    let third = match (&view.state, shown.pronouns.as_deref()) {
        (ProfileState::Loading, _) => Span::styled("Loading profile…", muted),
        (ProfileState::Failed(msg), _) => Span::styled(fit(msg, header_w), danger),
        (_, Some(p)) if !p.trim().is_empty() => Span::styled(fit(p, header_w), text),
        _ => Span::raw(""),
    };
    let fourth = match profile {
        Some(p) if p.profile_limited == Some(true) => {
            Span::styled(fit("Limited profile", header_w), muted)
        }
        _ => Span::raw(""),
    };
    // Rows three and four only when they say something, unless the
    // avatar needs the room.
    for span in [third, fourth] {
        if avatar || !span.content.is_empty() {
            lines.push(Line::from(vec![Span::raw(margin.clone()), span]));
        }
    }

    if let Some(bio) = shown.bio.as_deref().filter(|b| !b.trim().is_empty()) {
        lines.push(Line::from(""));
        for row in bio.trim_matches(['\n', '\r']).lines() {
            lines.push(Line::from(vec![
                Span::raw(" "),
                Span::styled(row.to_string(), text),
            ]));
        }
    }

    lines.push(Line::from(""));
    let label = |s: &str| Span::styled(format!(" {s:<18}"), dim);

    if let Some(d) = snowflake_date(&user.id) {
        lines.push(Line::from(vec![
            label("On Fluxer since"),
            Span::styled(d, text),
        ]));
    }
    if let Some(d) = member
        .as_ref()
        .and_then(|m| m.joined_at.as_deref())
        .and_then(iso_date)
    {
        let what = match guild_name.as_deref() {
            Some(g) => format!("Joined {}", fit(g, 40)),
            None => "Joined".to_string(),
        };
        lines.push(Line::from(vec![label(&what), Span::styled(d, text)]));
    }

    if let (Some(gid), Some(m)) = (gid, member.as_ref()) {
        let mut spans = vec![label("Roles")];
        let mut any = false;
        for (name, color) in role_names(app, gid, &m.roles) {
            let style = if color == 0 {
                pill
            } else {
                crate::ui::theme::pill_style(crate::ui::theme::rgb_pack_to_color(color))
            };
            spans.push(Span::styled(format!(" {name} "), style));
            spans.push(Span::raw(" "));
            any = true;
        }
        if any {
            lines.push(Line::from(spans));
        }
    }

    if let Some(p) = profile {
        if let Some(off) = p.timezone_offset {
            lines.push(Line::from(vec![
                label("Local time"),
                Span::styled(local_time(off, app.ui_settings.clock_12h), text),
            ]));
        }
        let premium = match p.premium_type {
            Some(2) => Some(match p.premium_lifetime_sequence {
                Some(n) => format!("Lifetime #{n}"),
                None => "Lifetime".to_string(),
            }),
            Some(1) => Some(match p.premium_since.as_deref().and_then(iso_date) {
                Some(d) => format!("Since {d}"),
                None => "Yes".to_string(),
            }),
            _ => None,
        };
        if let Some(text_value) = premium {
            lines.push(Line::from(vec![
                label("Premium"),
                Span::styled(text_value, text),
            ]));
        }
        if let Some(accounts) = p.connected_accounts.as_ref().filter(|a| !a.is_empty()) {
            for (i, a) in accounts.iter().enumerate() {
                let head = if i == 0 {
                    label("Connections")
                } else {
                    label("")
                };
                let mark = if a.verified { " ✓" } else { "" };
                lines.push(Line::from(vec![
                    head,
                    Span::styled(format!("{}: {}{mark}", a.kind, a.name), text),
                ]));
            }
        }
        if let Some(guilds) = p.mutual_guilds.as_ref().filter(|g| !g.is_empty()) {
            let names: Vec<String> = guilds
                .iter()
                .map(|g| {
                    app.guilds
                        .iter()
                        .find(|x| x.id == g.id)
                        .map(|x| x.name.clone())
                        .unwrap_or_else(|| "?".to_string())
                })
                .collect();
            lines.push(Line::from(vec![
                label("Mutual communities"),
                Span::styled(names.join(", "), text),
            ]));
        }
        if let Some(friends) = p.mutual_friends.as_ref().filter(|f| !f.is_empty()) {
            let names: Vec<String> = friends.iter().map(crate::app::display_name).collect();
            lines.push(Line::from(vec![
                label("Mutual friends"),
                Span::styled(names.join(", "), text),
            ]));
        }
    }

    lines
}

/// Badges as the web app shows them: account flags, the bot tag, premium.
fn badges(
    user: &crate::api::types::UserPartialResponse,
    profile: Option<&UserProfileResponse>,
) -> Vec<&'static str> {
    let mut out = Vec::new();
    if user.bot {
        out.push("BOT");
    }
    if user.system {
        out.push("SYSTEM");
    }
    if user.flags & user_flags::STAFF != 0 {
        out.push("Staff");
    }
    if user.flags & user_flags::PARTNER != 0 {
        out.push("Partner");
    }
    if user.flags & user_flags::BUG_HUNTER != 0 {
        out.push("Bug hunter");
    }
    match profile.and_then(|p| p.premium_type) {
        Some(2) => out.push("Lifetime premium"),
        Some(1) => out.push("Premium"),
        _ => {}
    }
    out
}

/// The member's roles, highest first, as (name, colour); ids the roster
/// does not know are skipped.
fn role_names(app: &App, guild_id: &str, role_ids: &[String]) -> Vec<(String, u32)> {
    let Some(roles) = app.guild_roles.get(guild_id) else {
        return Vec::new();
    };
    let mut found: Vec<&crate::api::types::GuildRoleResponse> = role_ids
        .iter()
        .filter_map(|id| roles.iter().find(|r| r.id.trim() == id.trim()))
        .filter(|r| r.id.trim() != guild_id.trim())
        .collect();
    found.sort_by_key(|r| std::cmp::Reverse(r.position));
    found
        .into_iter()
        .map(|r| (r.name.clone(), r.color))
        .collect()
}

/// A snowflake's creation day, in local time.
fn snowflake_date(id: &str) -> Option<String> {
    let id: u64 = id.parse().ok()?;
    let ms = (id >> 22) as i64 + FLUXER_EPOCH_MS;
    let when = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)?;
    Some(
        when.with_timezone(&chrono::Local)
            .format("%-d %b %Y")
            .to_string(),
    )
}

/// An ISO 8601 timestamp as a day, in local time.
fn iso_date(raw: &str) -> Option<String> {
    let when = chrono::DateTime::parse_from_rfc3339(raw).ok()?;
    Some(
        when.with_timezone(&chrono::Local)
            .format("%-d %b %Y")
            .to_string(),
    )
}

/// The time of day where the user is, from their offset in minutes.
fn local_time(offset_minutes: i32, clock_12h: bool) -> String {
    let now = chrono::Utc::now() + chrono::Duration::minutes(offset_minutes as i64);
    let clock = if clock_12h {
        now.format("%-I:%M %p").to_string()
    } else {
        now.format("%H:%M").to_string()
    };
    let sign = if offset_minutes < 0 { '-' } else { '+' };
    let abs = offset_minutes.unsigned_abs();
    let zone = if abs.is_multiple_of(60) {
        format!("UTC{sign}{}", abs / 60)
    } else {
        format!("UTC{sign}{}:{:02}", abs / 60, abs % 60)
    };
    format!("{clock} ({zone})")
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
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snowflake_dates_use_the_fluxer_epoch() {
        // 2015-01-01T00:00:00Z is the epoch itself.
        let d = snowflake_date(&(0u64 << 22).to_string()).unwrap();
        let expected = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(FLUXER_EPOCH_MS)
            .unwrap()
            .with_timezone(&chrono::Local)
            .format("%-d %b %Y")
            .to_string();
        assert_eq!(d, expected);
        assert!(snowflake_date("not a number").is_none());
    }

    #[test]
    fn local_time_names_the_zone() {
        assert!(local_time(120, false).ends_with("(UTC+2)"));
        assert!(local_time(-330, false).ends_with("(UTC-5:30)"));
        assert!(local_time(0, true).ends_with("(UTC+0)"));
    }

    #[test]
    fn fit_cuts_with_an_ellipsis() {
        assert_eq!(fit("short", 10), "short");
        assert_eq!(fit("a rather long name", 8), "a rathe…");
    }

    #[test]
    fn badges_follow_flags_and_premium() {
        let mut user = crate::api::types::UserPartialResponse::default();
        assert!(badges(&user, None).is_empty());
        user.flags = user_flags::STAFF | user_flags::BUG_HUNTER;
        user.bot = true;
        assert_eq!(badges(&user, None), vec!["BOT", "Staff", "Bug hunter"]);
        let profile = UserProfileResponse {
            premium_type: Some(2),
            ..Default::default()
        };
        assert_eq!(
            badges(&user, Some(&profile)).last(),
            Some(&"Lifetime premium")
        );
    }
}
