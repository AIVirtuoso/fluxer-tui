use crate::app::{App, Focus, ServerSelection};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let server = match &app.selected_server {
        ServerSelection::DirectMessages => "DMs".to_string(),
        ServerSelection::Guild(_) => app.selected_server_name(),
    };

    let mut status_mid = if app.status_message.is_empty() {
        String::new()
    } else {
        format!(" | {}", app.status_message)
    };
    if let Some(player) = app.audio.as_ref()
        && app.status_message.is_empty()
    {
        status_mid = format!(" | \u{266A} {}", player.label);
    }
    // a call, or one ringing, outranks the rest: it is the thing the
    // reader most needs to know is happening
    if let Some(voice) = app.voice_status_line()
        && app.status_message.is_empty()
    {
        status_mid = format!(" | \u{1F50A} {voice}");
    }

    let hints = match app.focus {
        Focus::Servers => " · j/k servers · n notifications · Tab/h/l · l open channels",
        Focus::Channels => " · j/k channels · n notifications · Enter msg · i input · R refresh",
        Focus::Messages => {
            if app.selected_message_index.is_some() {
                " · r/y reply/copy · f forward · e react · Ctrl+E edit · Ctrl+D del"
            } else {
                " · s select · Alt+A · i input · Ctrl+H help"
            }
        }
        Focus::Input => {
            if app.input_mark || app.input_selection().is_some() {
                " · selecting: Ctrl+C copy · Ctrl+X cut · Ctrl+B/I/S mark · Esc drop"
            } else {
                " · Alt+Enter/Ctrl+J newline · Ctrl+F file · Ctrl+V paste · Ctrl+K picker · F1 help"
            }
        }
    };

    let paragraph = Paragraph::new(Line::from(vec![
        Span::styled(" ", Style::default()),
        Span::styled(
            app.gateway_status.label(),
            crate::ui::theme::gateway_status_style(app.gateway_status),
        ),
        Span::styled(
            format!(" | {server}{status_mid}"),
            crate::ui::theme::dim_style(),
        ),
        Span::styled(hints, crate::ui::theme::muted_style()),
    ]))
    .style(Style::default().bg(crate::ui::theme::bg_tertiary()));
    frame.render_widget(paragraph, area);
}
