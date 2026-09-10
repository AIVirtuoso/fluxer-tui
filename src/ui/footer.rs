//! The bottom line of an overlay.
//!
//! Every overlay covers the whole frame, status bar included, so anything
//! `set_status` says while one is open used to be invisible: sending a
//! friend request told you nothing, because the words went behind the
//! list you were looking at. This line is where they go instead.
//!
//! It shows the keys that work here, and swaps to a notice for a few
//! seconds when something happens — so the reader is never left without
//! either the hints or the answer.

use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Draw the line: `app`'s status message when there is one, otherwise
/// `hints`.
pub fn render(frame: &mut Frame, area: Rect, app: &App, hints: &str) {
    frame.render_widget(
        Paragraph::new(line(app, hints)).alignment(Alignment::Center),
        area,
    );
}

/// The line itself, so a test can read it without a terminal.
pub fn line(app: &App, hints: &str) -> Line<'static> {
    match notice(app) {
        Some(notice) => Line::from(Span::styled(
            notice,
            Style::default()
                .fg(crate::ui::theme::accent())
                .add_modifier(Modifier::BOLD),
        )),
        None => Line::from(Span::styled(
            hints.to_string(),
            crate::ui::theme::muted_style(),
        )),
    }
}

/// A title for one of the autocomplete popups, with the keys on it.
///
/// The `:`, `@` and `/` popups sit straight on top of the compose box
/// and have no row to spare for a line of their own, so the keys go in
/// the title instead — the same trick the message pane uses to name the
/// author of the message at its top without costing a row.
///
/// The longest wording that fits is used, so a narrow popup still says
/// something rather than being cut off mid-word.
pub fn autocomplete_title(name: &str, width: u16) -> String {
    const FULL: &str = "\u{2191}\u{2193} move \u{00B7} Tab/Enter pick \u{00B7} Esc close";
    const SHORT: &str = "\u{2191}\u{2193} Tab Esc";
    // the two border columns are not the title's to use
    let usable = width.saturating_sub(2) as usize;
    for keys in [FULL, SHORT] {
        let title = format!(" {name} \u{00B7} {keys} ");
        if title.chars().count() <= usable {
            return title;
        }
    }
    format!(" {name} ")
}

/// What just happened, when anything did.
fn notice(app: &App) -> Option<String> {
    let message = app.status_message.trim();
    (!message.is_empty()).then(|| message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ServerSelection;
    use std::time::Duration;

    fn app() -> App {
        App::new(
            Default::default(),
            Default::default(),
            None,
            Vec::new(),
            Vec::new(),
            ServerSelection::DirectMessages,
            None,
            Default::default(),
        )
    }

    fn text(line: &Line<'static>) -> String {
        line.spans.iter().map(|s| s.content.to_string()).collect()
    }

    #[test]
    fn with_nothing_to_say_the_hints_are_shown() {
        let app = app();
        assert_eq!(
            text(&line(&app, "Enter go  ·  Esc close")),
            "Enter go  ·  Esc close"
        );
    }

    #[test]
    fn a_notice_takes_the_line_while_it_lasts() {
        let mut app = app();
        app.set_transient_status("Asked ada#0001 to be friends.", Duration::from_secs(4));
        assert_eq!(
            text(&line(&app, "Enter go")),
            "Asked ada#0001 to be friends."
        );
        // and once it has run out the hints come back
        app.set_transient_status("gone", Duration::from_millis(0));
        app.expire_status_if_needed();
        assert_eq!(text(&line(&app, "Enter go")), "Enter go");
    }

    #[test]
    fn a_popup_title_keeps_the_longest_wording_that_fits() {
        // the command popup is 64 wide and takes the lot
        let wide = autocomplete_title("/ commands", 64);
        assert!(wide.contains("Tab/Enter pick"), "{wide}");
        assert!(wide.chars().count() <= 62, "{wide}");

        // the emoji popup is 40 and falls back to the short form rather
        // than being cut off part way through a word
        let narrow = autocomplete_title("Emojis", 40);
        assert!(narrow.contains("Tab"), "{narrow}");
        assert!(!narrow.contains("pick"), "{narrow}");
        assert!(narrow.chars().count() <= 38, "{narrow}");

        // and a popup with no room for keys at all still says what it is
        let tiny = autocomplete_title("Emojis", 12);
        assert_eq!(tiny, " Emojis ");
    }

    #[test]
    fn a_notice_is_told_apart_from_the_hints_by_more_than_position() {
        let mut app = app();
        let hints = text(&line(&app, "Enter go"));
        app.set_status("Sent.");
        let notice = line(&app, "Enter go");
        assert_ne!(text(&notice), hints);
        // bold and accented, so it reads as an answer rather than a list
        // of keys, on a terminal with colour and without
        assert!(notice.spans[0].style.add_modifier.contains(Modifier::BOLD));
    }
}
