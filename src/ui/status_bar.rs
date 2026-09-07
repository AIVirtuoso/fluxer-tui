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

    let hints = match app.focus {
        Focus::Servers => " · j/k servers · n notifications · Tab/h/l · l open channels",
        Focus::Channels => " · j/k channels · n notifications · Enter msg · i input · R refresh",
        Focus::Messages => {
            if app.selected_message_index.is_some() {
                " · r reply · f forward · e react · Ctrl+E edit · Ctrl+D del · Alt+A"
            } else {
                " · s select · Alt+A · i input · Ctrl+H help"
            }
        }
        Focus::Input => {
            " · Ctrl+F file · Ctrl+V paste · Ctrl+K picker · Ctrl+N/P channel · Ctrl+H help"
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
