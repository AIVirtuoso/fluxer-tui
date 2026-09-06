use ratatui::style::{Color, Modifier, Style};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};

/// When set, every colour comes from the terminal's own palette (default
/// foreground/background plus the 16 ANSI colours) instead of the fixed
/// Fluxer RGB theme, so the client looks like the rest of the terminal.
static TERMINAL: AtomicBool = AtomicBool::new(true);

pub fn set_terminal_theme(on: bool) {
    TERMINAL.store(on, Ordering::Relaxed);
}

pub fn is_terminal_theme() -> bool {
    TERMINAL.load(Ordering::Relaxed)
}

macro_rules! colour {
    ($name:ident, $terminal:expr, $fluxer:expr) => {
        pub fn $name() -> Color {
            if is_terminal_theme() {
                $terminal
            } else {
                $fluxer
            }
        }
    };
}

colour!(bg, Color::Reset, Color::Rgb(30, 31, 34));
colour!(bg_secondary, Color::Reset, Color::Rgb(43, 45, 49));
colour!(bg_tertiary, Color::Reset, Color::Rgb(32, 34, 37));
colour!(accent, Color::Blue, Color::Rgb(88, 101, 242));
colour!(accent_dim, Color::Blue, Color::Rgb(71, 82, 196));
colour!(voice_color, Color::Green, Color::Rgb(87, 242, 135));
colour!(text, Color::Reset, Color::Rgb(219, 222, 225));
// In the terminal theme dim and muted text keep the default foreground and
// rely on the DIM attribute (see `dim_style` / `muted_style`): "bright black"
// is close to invisible on many dark palettes.
colour!(text_dim, Color::Reset, Color::Rgb(148, 155, 164));
colour!(text_muted, Color::Reset, Color::Rgb(94, 103, 114));
colour!(emoji_unknown, Color::Yellow, Color::Rgb(254, 231, 92));
colour!(link_color, Color::Cyan, Color::Rgb(0, 168, 252));
colour!(danger, Color::Red, Color::Rgb(237, 66, 69));
// Others typing (input bar title): distinct from `text_muted` so it does
// not match the empty placeholder.
colour!(typing_others, Color::Green, Color::Rgb(114, 218, 167));

const USERNAME_COLORS_FLUXER: [Color; 12] = [
    Color::Rgb(235, 69, 158),
    Color::Rgb(237, 66, 69),
    Color::Rgb(241, 196, 15),
    Color::Rgb(46, 204, 113),
    Color::Rgb(26, 188, 156),
    Color::Rgb(52, 152, 219),
    Color::Rgb(155, 89, 182),
    Color::Rgb(230, 126, 34),
    Color::Rgb(173, 20, 87),
    Color::Rgb(0, 131, 143),
    Color::Rgb(156, 204, 101),
    Color::Rgb(216, 67, 21),
];

const USERNAME_COLORS_TERMINAL: [Color; 12] = [
    Color::Magenta,
    Color::Red,
    Color::Yellow,
    Color::Green,
    Color::Cyan,
    Color::Blue,
    Color::LightMagenta,
    Color::LightRed,
    Color::LightYellow,
    Color::LightGreen,
    Color::LightCyan,
    Color::LightBlue,
];

pub fn username_color(id: &str) -> Color {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut hasher);
    let hash = hasher.finish();
    let palette = if is_terminal_theme() {
        &USERNAME_COLORS_TERMINAL
    } else {
        &USERNAME_COLORS_FLUXER
    };
    palette[(hash as usize) % palette.len()]
}

pub fn self_username_color() -> Color {
    if is_terminal_theme() {
        Color::LightCyan
    } else {
        Color::Rgb(0, 229, 255)
    }
}

/// Secondary text: timestamps, hints, section labels.
pub fn dim_style() -> Style {
    if is_terminal_theme() {
        Style::default().add_modifier(Modifier::DIM)
    } else {
        Style::default().fg(text_dim())
    }
}

/// Tertiary text: placeholders, unfocused chrome.
pub fn muted_style() -> Style {
    if is_terminal_theme() {
        Style::default().add_modifier(Modifier::DIM)
    } else {
        Style::default().fg(text_muted())
    }
}

/// Selected row in a popup list.
pub fn highlight_style() -> Style {
    if is_terminal_theme() {
        Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Black)
            .bg(accent())
            .add_modifier(Modifier::BOLD)
    }
}

/// @mention / #channel pills: a coloured block in the Fluxer theme, bold
/// coloured text on the terminal's own background otherwise.
pub fn pill_style(colour: Color) -> Style {
    if is_terminal_theme() {
        Style::default().fg(colour).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Black).bg(colour)
    }
}

/// Inline `code` and code blocks.
pub fn code_style() -> Style {
    if is_terminal_theme() {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(text()).bg(bg_tertiary())
    }
}

pub fn rgb_pack_to_color(packed: u32) -> Color {
    Color::Rgb(
        ((packed >> 16) & 0xFF) as u8,
        ((packed >> 8) & 0xFF) as u8,
        (packed & 0xFF) as u8,
    )
}

pub fn role_mention_style(color: u32) -> Style {
    if color == 0 {
        Style::default().fg(accent()).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(rgb_pack_to_color(color))
    }
}

pub fn focused_border(focused: bool) -> Style {
    if focused {
        Style::default().fg(accent())
    } else {
        muted_style()
    }
}

pub fn gateway_status_style(status: crate::app::GatewayStatus) -> Style {
    use crate::app::GatewayStatus::*;
    match status {
        Connecting | Reconnecting => Style::default().fg(Color::Yellow),
        Connected => Style::default().fg(voice_color()),
        Disconnected => Style::default().fg(danger()),
    }
}

/// The background colour as RGB when the theme fixes one (the Fluxer
/// theme); None on the terminal theme, whose background is the terminal's.
pub fn bg_rgb() -> Option<[u8; 3]> {
    match bg() {
        Color::Rgb(r, g, b) => Some([r, g, b]),
        _ => None,
    }
}
