use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
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
    let accent_soft = Style::default().fg(crate::ui::theme::accent_dim());
    let text = Style::default().fg(crate::ui::theme::text());
    let dim = crate::ui::theme::dim_style();
    let muted = crate::ui::theme::muted_style();
    let panel = Style::default()
        .fg(crate::ui::theme::text())
        .bg(crate::ui::theme::bg_secondary());

    let clock_on = app.ui_settings.clock_12h;
    let clock_primary = if clock_on {
        ("12-hour", "AM / PM")
    } else {
        ("24-hour", "00:00 – 23:59")
    };
    let clock_alt = if clock_on {
        ("24-hour", "00:00 – 23:59")
    } else {
        ("12-hour", "AM / PM")
    };
    let typing_on = app.ui_settings.show_typing_indicators;
    let typing_primary = if typing_on {
        ("On", "Who is typing, in the compose box's bottom edge")
    } else {
        ("Off", "Nothing about who is typing")
    };
    let typing_alt = if typing_on {
        ("Off", "Nothing about who is typing")
    } else {
        ("On", "Who is typing, in the compose box's bottom edge")
    };
    let perf_on = app.ui_settings.performance_mode;
    let perf_primary = if perf_on {
        ("On", "No pictures or animations, fewer redraws")
    } else {
        ("Off", "Full feature set")
    };
    let perf_alt = if perf_on {
        ("Off", "Full feature set")
    } else {
        ("On", "No pictures or animations, fewer redraws")
    };
    let inline_on = app.ui_settings.inline_media;
    let inline_primary = if inline_on {
        ("On", "Pictures and GIFs shown under messages")
    } else {
        ("Off", "Only file names; Ctrl+O opens them")
    };
    let inline_alt = if inline_on {
        ("Off", "Only file names; Ctrl+O opens them")
    } else {
        ("On", "Pictures and GIFs shown under messages")
    };
    let avatars_on = app.ui_settings.avatars;
    let avatars_primary = if avatars_on {
        ("On", "Profile pictures beside messages")
    } else {
        ("Off", "No profile pictures")
    };
    let avatars_alt = if avatars_on {
        ("Off", "No profile pictures")
    } else {
        ("On", "Profile pictures beside messages")
    };
    let terminal_theme = app.ui_settings.theme == crate::config::Theme::Terminal;
    let theme_primary = if terminal_theme {
        ("Terminal", "Use the terminal's own colours")
    } else {
        ("Fluxer", "Fixed dark theme like the web app")
    };
    let theme_alt = if terminal_theme {
        ("Fluxer", "Fixed dark theme like the web app")
    } else {
        ("Terminal", "Use the terminal's own colours")
    };

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("  ", text),
        Span::styled("fluxer-tui", accent),
        Span::styled(" · preferences", dim),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        "  INTERFACE",
        accent_soft.add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(vec![Span::styled(
        "  ───────────────────────────────────────────",
        accent_soft,
    )]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled("  Message timestamps", dim)]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  ", text),
        Span::styled(
            if app.settings_cursor == 0 {
                "▸ "
            } else {
                "  "
            },
            if app.settings_cursor == 0 {
                accent
            } else {
                muted
            },
        ),
        Span::styled(
            format!("{}  ·  {}", clock_primary.0, clock_primary.1),
            panel.add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("    ", text),
        Span::styled(format!("{}  ·  {}", clock_alt.0, clock_alt.1), muted),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled("  Typing indicators", dim)]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  ", text),
        Span::styled(
            if app.settings_cursor == 1 {
                "▸ "
            } else {
                "  "
            },
            if app.settings_cursor == 1 {
                accent
            } else {
                muted
            },
        ),
        Span::styled(
            format!("{}  ·  {}", typing_primary.0, typing_primary.1),
            panel.add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("    ", text),
        Span::styled(format!("{}  ·  {}", typing_alt.0, typing_alt.1), muted),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        "  Performance Mode (Low Spec)",
        dim,
    )]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  ", text),
        Span::styled(
            if app.settings_cursor == 2 {
                "▸ "
            } else {
                "  "
            },
            if app.settings_cursor == 2 {
                accent
            } else {
                muted
            },
        ),
        Span::styled(
            format!("{}  ·  {}", perf_primary.0, perf_primary.1),
            panel.add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("    ", text),
        Span::styled(format!("{}  ·  {}", perf_alt.0, perf_alt.1), muted),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled("  Colours", dim)]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  ", text),
        Span::styled(
            if app.settings_cursor == 3 {
                "▸ "
            } else {
                "  "
            },
            if app.settings_cursor == 3 {
                accent
            } else {
                muted
            },
        ),
        Span::styled(
            format!("{}  ·  {}", theme_primary.0, theme_primary.1),
            panel.add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("    ", text),
        Span::styled(format!("{}  ·  {}", theme_alt.0, theme_alt.1), muted),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled("  Pictures in chat", dim)]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  ", text),
        Span::styled(
            if app.settings_cursor == 4 {
                "▸ "
            } else {
                "  "
            },
            if app.settings_cursor == 4 {
                accent
            } else {
                muted
            },
        ),
        Span::styled(
            format!("{}  ·  {}", inline_primary.0, inline_primary.1),
            panel.add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("    ", text),
        Span::styled(format!("{}  ·  {}", inline_alt.0, inline_alt.1), muted),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled("  Profile pictures", dim)]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  ", text),
        Span::styled(
            if app.settings_cursor == 5 {
                "▸ "
            } else {
                "  "
            },
            if app.settings_cursor == 5 {
                accent
            } else {
                muted
            },
        ),
        Span::styled(
            format!("{}  ·  {}", avatars_primary.0, avatars_primary.1),
            panel.add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("    ", text),
        Span::styled(format!("{}  ·  {}", avatars_alt.0, avatars_alt.1), muted),
    ]));
    use crate::config::NotifyMode;
    let notify_label = |m: NotifyMode| match m {
        NotifyMode::Auto => ("Auto", "notify-send where there is a display"),
        NotifyMode::Desktop => ("Desktop", "notify-send (libnotify)"),
        NotifyMode::Mail => ("Mail", "GNU mail to the login user (for the console)"),
        NotifyMode::Off => ("Off", "No notifications outside the client"),
    };
    let notify_next = match app.ui_settings.notifications {
        NotifyMode::Auto => NotifyMode::Desktop,
        NotifyMode::Desktop => NotifyMode::Mail,
        NotifyMode::Mail => NotifyMode::Off,
        NotifyMode::Off => NotifyMode::Auto,
    };
    let notify_primary = notify_label(app.ui_settings.notifications);
    let notify_alt = notify_label(notify_next);
    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        "  Notifications (mentions and direct messages)",
        dim,
    )]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  ", text),
        Span::styled(
            if app.settings_cursor == 6 {
                "▸ "
            } else {
                "  "
            },
            if app.settings_cursor == 6 {
                accent
            } else {
                muted
            },
        ),
        Span::styled(
            format!("{}  ·  {}", notify_primary.0, notify_primary.1),
            panel.add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("    ", text),
        Span::styled(
            format!("next: {}  ·  {}", notify_alt.0, notify_alt.1),
            muted,
        ),
    ]));
    let sound_primary = if app.ui_settings.notify_sound {
        ("On", "A sound with every notification")
    } else {
        ("Off", "Notifications are silent")
    };
    let sound_alt = if app.ui_settings.notify_sound {
        ("Off", "Notifications are silent")
    } else {
        ("On", "A sound with every notification")
    };
    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled("  Notification sound", dim)]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  ", text),
        Span::styled(
            if app.settings_cursor == 7 {
                "▸ "
            } else {
                "  "
            },
            if app.settings_cursor == 7 {
                accent
            } else {
                muted
            },
        ),
        Span::styled(
            format!("{}  ·  {}", sound_primary.0, sound_primary.1),
            panel.add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("    ", text),
        Span::styled(format!("{}  ·  {}", sound_alt.0, sound_alt.1), muted),
    ]));
    let block = Block::default()
        .title(Line::from(Span::styled(" Settings ", accent)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(crate::ui::theme::accent_dim()));

    // more rows than the terminal has lines: keep the selected row and
    // its alternative in view
    let selected = lines
        .iter()
        .position(|l| l.spans.iter().any(|s| s.content.as_ref() == "▸ "))
        .unwrap_or(0);
    let inner = content.height.saturating_sub(2) as usize;
    let scroll = (selected + 2).saturating_sub(inner) as u16;
    let paragraph = Paragraph::new(Text::from(lines))
        .block(block)
        .wrap(Wrap { trim: true })
        .scroll((scroll, 0))
        .alignment(Alignment::Left);

    frame.render_widget(paragraph, content);

    let hint = Paragraph::new(Line::from(vec![Span::styled(
        "↑/↓ move  ·  Space / Enter toggle  ·  Esc / q close",
        muted,
    )]))
    .alignment(Alignment::Center);
    frame.render_widget(hint, body[1]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NotifyMode;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn drawn(app: &App) -> String {
        let mut t = Terminal::new(TestBackend::new(90, 44)).unwrap();
        t.draw(|f| render(f, f.area(), app)).unwrap();
        let buf = t.backend().buffer().clone();
        (0..44)
            .map(|y| {
                (0..90)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_notifications_row_shows_the_mode_and_cycles_through_all_four() {
        let mut app = App::new(
            Default::default(),
            Default::default(),
            None,
            Vec::new(),
            Vec::new(),
            crate::app::ServerSelection::DirectMessages,
            None,
            Default::default(),
        );
        app.settings_cursor = 6;
        assert_eq!(app.ui_settings.notifications, NotifyMode::Auto);
        let s = drawn(&app);
        assert!(
            s.contains("Notifications (mentions and direct messages)"),
            "{s}"
        );
        assert!(
            s.contains("▸ Auto  ·  notify-send where there is a display"),
            "{s}"
        );
        let mut seen = vec![app.ui_settings.notifications];
        for _ in 0..3 {
            app.toggle_settings_selection();
            seen.push(app.ui_settings.notifications);
        }
        assert_eq!(
            seen,
            [
                NotifyMode::Auto,
                NotifyMode::Desktop,
                NotifyMode::Mail,
                NotifyMode::Off
            ]
        );
        assert!(drawn(&app).contains("▸ Off  ·  No notifications outside the client"));
        app.toggle_settings_selection();
        assert_eq!(app.ui_settings.notifications, NotifyMode::Auto);
    }

    #[test]
    fn the_sound_row_is_last_and_toggles() {
        let mut app = App::new(
            Default::default(),
            Default::default(),
            None,
            Vec::new(),
            Vec::new(),
            crate::app::ServerSelection::DirectMessages,
            None,
            Default::default(),
        );
        app.settings_cursor = App::UI_SETTINGS_LAST_ROW;
        assert!(app.ui_settings.notify_sound);
        let s = drawn(&app);
        assert!(s.contains("Notification sound"), "{s}");
        assert!(
            s.contains("▸ On  ·  A sound with every notification"),
            "{s}"
        );
        app.toggle_settings_selection();
        assert!(!app.ui_settings.notify_sound);
        assert!(drawn(&app).contains("▸ Off  ·  Notifications are silent"));
        // a short terminal scrolls the selected row into view
        let mut t = Terminal::new(TestBackend::new(90, 24)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let buf = t.backend().buffer().clone();
        let short = (0..24)
            .map(|y| {
                (0..90)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            short.contains("▸ Off  ·  Notifications are silent"),
            "{short}"
        );
        assert!(short.contains("On  ·  A sound with every notification"));
        app.settings_cursor = 0;
        assert!(drawn(&app).contains("▸ 24-hour"));
    }
}
