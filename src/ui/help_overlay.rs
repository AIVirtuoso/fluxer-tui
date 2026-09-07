use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

const HELP: &str = r#"Global (almost any screen)
  F1 - keybindings (this overlay)
  F2 - settings (UI preferences; saved to config)
  Ctrl+H - keybindings when focus is not the message input
  Ctrl+C - quit          Ctrl+L - log out and quit
  q - quit (when not typing in input)

Focus & navigation
  Tab / Shift+Tab - cycle: servers → channels → messages → input
  h / l / Left / Right - same as Tab (previous / next focus)
  Esc - channels focus; clears message selection; closes overlays
  i - jump to input (text channel with send permission)
  Enter (channels) - open messages, or open link-channel URL, or focus input on text channel
  R - refresh: reload current channel messages and guild channels/members

Servers (left column)
  Up / Down / j / k - move server selection
  n - notification settings for the selected community
  l / Right - open channel list for selected server

Channels (middle column)
  Up / Down / j / k - move channel
  n - notification settings for the selected community
  Enter - open message view for channel

Messages
  Up / Down / j / k - scroll list, or move selection when a message is selected
  PgUp / PgDn - scroll message pane
  G - jump to the newest message; while scrolled up, the view stays put as messages arrive
  Scroll up near the top - older messages load automatically
  s - select last message (selection mode)
  r - reply to selected message
  p - profile of the selected message's author (p again: picture full size; Esc closes)
  f - forward selected (pick channel with Ctrl+K, optional note, Enter)
  e - react: opens emoji picker on selected message (Enter to send reaction, Esc cancels)
  Ctrl+E - edit your message (focuses input; Enter save, Esc cancel)
  Ctrl+D - delete selected (yours, or mod with Manage Messages)
  Ctrl+O - the selected message's picture or GIF full size in-terminal (the chat shows
           previews); videos open via the system default app; audio plays through
           mpv/ffplay/pw-play/paplay/aplay or [media] audio_player (again: stop)

Input
  Enter - send; save edit; send forward with reference
  Backspace - delete character
  Ctrl+Backspace / Ctrl+H - delete previous word (whitespace-separated chunk)
  Ctrl+U - clear input
  Ctrl+F or /attach - file picker: any kind of file, with a preview of pictures and videos
  /attach <path> - attach a file by path; Ctrl+V - the image or the files on the clipboard
  Ctrl+O - the last staged picture or video full size; Ctrl+X - drop the last staged file
  Enter sends text and attachments together (the compose box shows what is staged)
  Up - back to message list
  : - custom emoji autocomplete     @ - mention autocomplete (guild/DM)
  Long lines wrap; input height grows with wrapped rows

Channel picker (Ctrl+K)
  Type to filter   Up/Down - move   Enter - jump   Esc - close   Backspace

Ctrl+channel (disabled while : or @ autocomplete is open)
  Ctrl+N / Ctrl+P - next / previous text channel (wraps)
  Ctrl+K - channel picker
  Ctrl+E / Ctrl+D - edit / delete selected message (messages focus + selection)
  Ctrl+O - full-size picture when a message is selected (see Messages)

Alt
  Alt+A - next channel with unread or mention (hotlist; wraps)

Other
  Edited messages show “(edited)” after the timestamp when the API sends edited_timestamp.
"#;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    frame.render_widget(Clear, area);
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(4),
            Constraint::Length(1),
        ])
        .split(area);

    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(20),
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
    let text_style = Style::default().fg(crate::ui::theme::text());
    let lines: Vec<Line> = HELP
        .lines()
        .map(|l| Line::from(Span::styled(l.to_string(), text_style)))
        .collect();
    let help_text = Text::from(lines);

    let inner_w = content.width.saturating_sub(2).max(1);
    let inner_h = content.height.saturating_sub(2).max(1);

    let line_total = Paragraph::new(help_text.clone())
        .wrap(Wrap { trim: true })
        .line_count(inner_w)
        .max(1) as u16;
    let max_scroll = line_total.saturating_sub(inner_h);
    let scroll_y = app.help_scroll.min(max_scroll);

    let block = Block::default()
        .title(Line::from(Span::styled(
            " Keybindings ",
            Style::default()
                .fg(crate::ui::theme::accent())
                .add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(crate::ui::theme::accent_dim()));

    let paragraph = Paragraph::new(help_text)
        .block(block)
        .wrap(Wrap { trim: true })
        .scroll((scroll_y, 0))
        .alignment(Alignment::Left);

    frame.render_widget(paragraph, content);

    let footer = if max_scroll > 0 {
        " ↑/↓ PgUp/PgDn - scroll · Esc / Enter / q - close "
    } else {
        " Esc / Enter / q - close "
    };

    let hint = Paragraph::new(Line::from(Span::styled(
        footer,
        crate::ui::theme::muted_style(),
    )))
    .alignment(Alignment::Center);
    frame.render_widget(hint, body[1]);
}
