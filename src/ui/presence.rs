//! Drawing somebody's online state.
//!
//! Two glyphs carry it rather than four: a filled circle for anybody who
//! is about and a hollow one for anybody who is not, with the colour
//! separating online from idle from do-not-disturb. That is on purpose.
//! The shape carries the distinction that matters at a glance and is the
//! one a monochrome terminal can still see, and both characters are in
//! every monospace font worth the name — where a fourth, rarer glyph
//! would come out as a missing-character box on some of them (xterm's
//! Monospace already does that to U+2800; see the fork's notes).
//!
//! Anywhere the state is worth spelling out — the profile, the status
//! bar — it is written in words as well, so colour is never the only
//! thing saying it.

use crate::api::types::PresenceStatus;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

/// U+25CF, a filled circle: they are about.
const HERE: &str = "\u{25CF}";
/// U+25CB, a hollow circle: they are not.
const AWAY: &str = "\u{25CB}";

pub fn glyph(status: PresenceStatus) -> &'static str {
    if status.is_offline() { AWAY } else { HERE }
}

pub fn colour(status: PresenceStatus) -> Color {
    match status {
        PresenceStatus::Online => crate::ui::theme::voice_color(),
        PresenceStatus::Idle => crate::ui::theme::mention_bar(),
        PresenceStatus::Dnd => crate::ui::theme::danger(),
        PresenceStatus::Invisible | PresenceStatus::Offline => crate::ui::theme::text_muted(),
    }
}

pub fn style(status: PresenceStatus) -> Style {
    let base = Style::default().fg(colour(status));
    if status.is_offline() {
        base.add_modifier(Modifier::DIM)
    } else {
        base
    }
}

/// The dot on its own, for a row that has no room to say more.
pub fn dot(status: PresenceStatus) -> Span<'static> {
    Span::styled(glyph(status), style(status))
}

/// The dot and a trailing space, for putting in front of a name.
pub fn dot_prefix(status: PresenceStatus) -> Span<'static> {
    Span::styled(format!("{} ", glyph(status)), style(status))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_two_glyphs_are_used_and_offline_is_the_hollow_one() {
        assert_eq!(glyph(PresenceStatus::Online), HERE);
        assert_eq!(glyph(PresenceStatus::Idle), HERE);
        assert_eq!(glyph(PresenceStatus::Dnd), HERE);
        assert_eq!(glyph(PresenceStatus::Offline), AWAY);
        // invisible is the reader's own way of looking offline, and looks
        // it here too
        assert_eq!(glyph(PresenceStatus::Invisible), AWAY);
    }

    #[test]
    fn each_state_has_its_own_colour_except_the_two_that_mean_away() {
        let colours = [
            colour(PresenceStatus::Online),
            colour(PresenceStatus::Idle),
            colour(PresenceStatus::Dnd),
        ];
        assert_eq!(
            colours
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            3
        );
        assert_eq!(
            colour(PresenceStatus::Offline),
            colour(PresenceStatus::Invisible)
        );
    }
}
