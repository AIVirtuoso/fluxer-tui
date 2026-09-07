//! The debug panel: the facts a bug report needs and the last lines of
//! the debug log, all of it free of message text and names (see
//! `crate::debug`).

use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

/// The facts shown at the top: what was known at start, then what is the
/// case now.
pub fn facts(app: &App) -> Vec<(String, String)> {
    let mut out = app.debug_facts.clone();
    let uptime = app.started_at.elapsed().as_secs();
    out.push((
        "uptime".into(),
        format!(
            "{}h {:02}m {:02}s",
            uptime / 3600,
            uptime / 60 % 60,
            uptime % 60
        ),
    ));
    out.push(("gateway".into(), app.gateway_status.label().to_string()));
    out.push((
        "loaded".into(),
        format!(
            "{} communities, {} channels, {} messages, {} custom emoji",
            app.guilds.len(),
            app.guild_channels.values().map(Vec::len).sum::<usize>() + app.private_channels.len(),
            app.messages.values().map(|m| m.len()).sum::<usize>(),
            app.custom_emojis.len()
        ),
    ));
    if let Some(gid) = app.active_guild_id() {
        let members = app.guild_members.get(&gid).map_or(0, Vec::len);
        let synced = app.guild_members_synced.contains(&gid);
        out.push((
            "members".into(),
            format!(
                "{members} loaded for the open community{}",
                if synced { " (complete)" } else { "" }
            ),
        ));
    }
    out.push((
        "settings".into(),
        format!(
            "{} theme, performance mode {}, pictures {}",
            if crate::ui::theme::is_terminal_theme() {
                "terminal"
            } else {
                "fluxer"
            },
            if app.ui_settings.performance_mode {
                "on"
            } else {
                "off"
            },
            if app.pictures_enabled() { "on" } else { "off" }
        ),
    ));
    out.push(("last frame".into(), format!("{} ms", app.last_frame_ms)));
    out.push((
        "debug log".into(),
        match crate::debug::path() {
            Some(p) => crate::debug::scrub_path(&p.display().to_string()),
            None => "off (start with --debug to keep one)".into(),
        },
    ));
    out
}

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
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
            Constraint::Min(42),
            Constraint::Length(2),
        ])
        .split(outer[1]);
    let popup = mid[1];
    frame.render_widget(Clear, popup);

    let accent = Style::default()
        .fg(crate::ui::theme::accent())
        .add_modifier(Modifier::BOLD);
    let dim = crate::ui::theme::dim_style();
    let muted = crate::ui::theme::muted_style();
    let text = Style::default().fg(crate::ui::theme::text());

    let mut lines: Vec<Line> = Vec::new();
    for (name, value) in facts(app) {
        lines.push(Line::from(vec![
            Span::styled(format!("  {name:<11} "), dim),
            Span::styled(value, text),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  Recent log", accent)));
    let recent = crate::debug::recent(crate::debug::RING_LINES);
    if recent.is_empty() {
        lines.push(Line::from(Span::styled("  (nothing yet)", muted)));
    }
    for entry in recent {
        lines.push(Line::from(Span::styled(format!("  {entry}"), text)));
    }

    let block = Block::default()
        .title(Line::from(Span::styled(" Debug ", accent)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(crate::ui::theme::accent_dim()));
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    let inner_h = popup.height.saturating_sub(2).max(1);
    let line_count = paragraph.line_count(popup.width).max(1) as u16;
    let max_scroll = line_count.saturating_sub(inner_h);
    let scroll = app.debug_scroll.min(max_scroll);
    frame.render_widget(paragraph.scroll((scroll, 0)), popup);

    let hint = Paragraph::new(Line::from(Span::styled(
        " j/k or ↑/↓ scroll · Home: facts · End: newest · s: save to a file · Esc / q close ",
        muted,
    )))
    .alignment(Alignment::Center);
    frame.render_widget(hint, outer[2]);
}
