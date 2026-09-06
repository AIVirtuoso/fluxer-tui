use image::DynamicImage;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui_image::picker::Picker;
use ratatui_image::protocol::{Protocol, StatefulProtocol};
use std::cell::RefCell;

use crate::api::types::{
    CHANNEL_DM, CHANNEL_DM_PERSONAL_NOTES, CHANNEL_GROUP_DM, CHANNEL_GUILD_CATEGORY,
    CHANNEL_GUILD_LINK, CHANNEL_GUILD_TEXT, CHANNEL_GUILD_VOICE, ChannelResponse,
    GuildMemberResponse, GuildResponse, MESSAGE_NOTIFICATIONS_ALL_MESSAGES,
    MESSAGE_NOTIFICATIONS_INHERIT, MESSAGE_NOTIFICATIONS_NO_MESSAGES,
    MESSAGE_NOTIFICATIONS_ONLY_MENTIONS, MessageResponse, ReadStateResponse, Snowflake,
    UserGuildChannelOverride, UserGuildMuteConfig, UserGuildSettingsPatch,
    UserGuildSettingsResponse, UserPartialResponse, UserPrivateResponse, UserSettingsResponse,
    VoiceStateResponse, WellKnownFluxerResponse, merge_user_cache, snowflake_sort_key,
};
use crate::config::UiSettings;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Servers,
    Channels,
    Messages,
    Input,
}

impl Focus {
    pub fn next(self) -> Self {
        match self {
            Self::Servers => Self::Channels,
            Self::Channels => Self::Messages,
            Self::Messages => Self::Input,
            Self::Input => Self::Servers,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Self::Servers => Self::Input,
            Self::Channels => Self::Servers,
            Self::Messages => Self::Channels,
            Self::Input => Self::Messages,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerSelection {
    DirectMessages,
    Guild(String),
}

impl ServerSelection {
    pub fn id(&self) -> String {
        match self {
            Self::DirectMessages => "@me".to_string(),
            Self::Guild(id) => id.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayStatus {
    Connecting,
    Connected,
    Reconnecting,
    Disconnected,
}

impl GatewayStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Reconnecting => "reconnecting",
            Self::Disconnected => "disconnected",
        }
    }
}

#[derive(Debug, Clone)]
pub struct EmojiMatch {
    pub label: String,
    pub insert: String,
    pub is_custom: bool,
    /// Guild emoji id, so the popup can draw its picture.
    pub custom_id: Option<String>,
    pub custom_animated: bool,
}

#[derive(Debug, Clone)]
pub struct EmojiAutocomplete {
    pub matches: Vec<EmojiMatch>,
    pub selected_index: usize,
}

/// One selectable row in @ autocomplete (users vs roles are separate insert targets).
#[derive(Debug, Clone)]
pub enum MentionPick {
    User {
        user_id: String,
        display: String,
        username: String,
    },
    Role {
        role_id: String,
        name: String,
        color: u32,
    },
}

impl MentionPick {
    fn matches_filter(&self, query: &str) -> bool {
        match self {
            MentionPick::User {
                display, username, ..
            } => display.to_lowercase().contains(query) || username.to_lowercase().contains(query),
            MentionPick::Role { name, .. } => name.to_lowercase().contains(query),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MentionAutocomplete {
    pub pool: Vec<MentionPick>,
    pub matches: Vec<usize>,
    pub selected_index: usize,
}

#[derive(Debug, Clone)]
pub struct CommandAutocomplete {
    pub matches: Vec<usize>,
    pub selected_index: usize,
}

pub const MAX_ATTACHMENTS_PER_MESSAGE: usize = 10;

/// Width in cells of an inline custom emoji (one row tall).
pub const CUSTOM_EMOJI_CELLS: u16 = 2;
/// Two braille blanks: not whitespace, so wrapping never trims them, and
/// invisible if the picture fails to land on top.
pub const CUSTOM_EMOJI_PLACEHOLDER: &str = "\u{2800}\u{2800}";

/// A guild emoji as it is known to the message renderer.
pub enum CustomEmojiState {
    Loading,
    Ready(PictureFrames),
    Failed,
}

/// One encoded picture per animation frame (a single frame for still
/// emoji). Animated emoji all run on the app's shared clock, see
/// [`App::custom_emoji_current`].
/// A picture, encoded for the terminal's graphics protocol or kept as
/// pixels for console mode.
#[derive(Clone)]
pub enum Picture {
    Terminal(std::sync::Arc<TerminalPicture>),
    Pixels(std::sync::Arc<image::RgbaImage>),
}

/// A picture as escape sequences for the terminal's graphics protocol,
/// ready to print at a block's cells: sixel and iTerm2 draw the whole
/// picture from the first row, kitty places one row at a time. The
/// sequences never enter ratatui's buffer: it would take their length for
/// the width of a character and re-send the rest of the screen on every
/// frame. The backend prints them where a sentinel cell sits instead.
pub struct TerminalPicture {
    /// (row within the block, what to print at that row's first cell).
    pub rows: Vec<(u16, std::sync::Arc<str>)>,
    pub area: Rect,
}

/// Encode a protocol picture for printing: draw it into a scratch buffer
/// the way ratatui-image would and lift out what it put in the cells.
pub fn terminal_picture(protocol: &Protocol) -> Option<TerminalPicture> {
    let area = protocol.area();
    if area.width == 0 || area.height == 0 {
        return None;
    }
    let mut buf = ratatui::buffer::Buffer::empty(Rect::new(0, 0, area.width, area.height));
    ratatui::widgets::Widget::render(ratatui_image::Image::new(protocol), buf.area, &mut buf);
    let mut rows = Vec::new();
    for y in 0..area.height {
        let cell = &buf[(0, y)];
        if cell.skip || cell.symbol().len() <= 1 {
            continue;
        }
        rows.push((y, std::sync::Arc::<str>::from(cell.symbol())));
    }
    if rows.is_empty() {
        return None;
    }
    Some(TerminalPicture { rows, area })
}

/// Numbers each set of frames, so a sentinel cell can tell pictures apart.
static PICTURE_SERIAL: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(1);

#[derive(Clone)]
pub struct PictureFrames {
    pub frames: Vec<Picture>,
    pub delays: Vec<Duration>,
    pub total: Duration,
    /// Tells this picture from any other one that was on the same cells.
    pub serial: u16,
    /// Frame index drawn last, when it first appeared, and the draw (frame
    /// of the UI) it was decided in: every instance of the emoji on screen
    /// in one draw shows the same frame.
    shown: std::cell::Cell<usize>,
    shown_at: std::cell::Cell<Option<Instant>>,
    decided_in_draw: std::cell::Cell<u64>,
}

impl PictureFrames {
    pub fn new(frames: Vec<Picture>, delays: Vec<Duration>) -> Self {
        let total = delays.iter().sum();
        Self {
            frames,
            delays,
            total,
            serial: PICTURE_SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            shown: std::cell::Cell::new(0),
            shown_at: std::cell::Cell::new(None),
            decided_in_draw: std::cell::Cell::new(0),
        }
    }

    pub fn is_animated(&self) -> bool {
        self.delays.len() > 1 && !self.total.is_zero()
    }

    /// The shortest delay between two frames.
    pub fn min_delay(&self) -> Option<Duration> {
        self.delays.iter().copied().min()
    }

    /// The frame to draw at `now`, in UI draw number `draw`. The clock only
    /// runs forward: from the frame shown last and the moment it went up,
    /// the frames whose delay has passed are stepped over, carrying the
    /// remainder, so the cadence stays exact however irregular the draws
    /// are and a frame is never shown again once it is over. A loop shorter
    /// than the time between two draws would land on the same frame; it is
    /// moved one on so it visibly plays. Decided once per draw: every
    /// instance on screen shows the same frame.
    pub fn index_at(&self, now: Instant, draw: u64) -> usize {
        if !self.is_animated() {
            return 0;
        }
        if self.decided_in_draw.get() == draw && self.shown_at.get().is_some() {
            return self.shown.get();
        }
        self.decided_in_draw.set(draw);
        let n = self.delays.len();
        let shown = self.shown.get();
        let Some(mut at) = self.shown_at.get() else {
            self.shown_at.set(Some(now));
            return shown;
        };
        let mut elapsed = now.saturating_duration_since(at);
        let mut stepped = false;
        if elapsed >= self.total {
            // whole loops went by (the pane was hidden, say): keep the phase
            elapsed = Duration::from_nanos((elapsed.as_nanos() % self.total.as_nanos()) as u64);
            at = now - elapsed;
            stepped = true;
        }
        let mut i = shown;
        while elapsed >= self.delays[i] {
            elapsed -= self.delays[i];
            at += self.delays[i];
            i = (i + 1) % n;
            stepped = true;
        }
        if stepped && i == shown {
            i = (i + 1) % n;
        }
        self.shown.set(i);
        self.shown_at.set(Some(at));
        i
    }

    /// The frame to draw now: its index and its picture.
    pub fn current(&self, now: Instant, draw: u64) -> (usize, &Picture) {
        let i = self.index_at(now, draw).min(self.frames.len() - 1);
        (i, &self.frames[i])
    }
}

/// Animation frames kept per custom emoji; enough for the usual short loops.
pub const CUSTOM_EMOJI_MAX_FRAMES: usize = 48;

impl std::fmt::Debug for PictureFrames {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PictureFrames({} frames)", self.frames.len())
    }
}

/// Where a reader scrolled up in the history is: the message under the
/// pane's top row and how many rows into it the top row lies. When
/// messages arrive or load while they read, the view is put back on that
/// message instead of drifting with the bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneAnchor {
    pub channel: Option<String>,
    pub message_id: String,
    pub offset: i64,
    /// The content's rows when the anchor was taken: a change means the
    /// content moved under the reader.
    pub total_rows: u32,
}

/// The message pane's view in one draw: which channel, where on screen,
/// how many rows its content had and how far it was scrolled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneView {
    pub channel: Option<String>,
    pub inner: Rect,
    pub total_rows: u16,
    pub top: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    /// A preview under a message.
    Picture,
    /// A profile picture beside a message.
    Avatar,
}

/// A block of cells showing a picture: what to fetch, at what size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaSlot {
    /// Cache key: the URL and the block size, since pictures are prepared
    /// for one exact size.
    pub key: String,
    pub url: String,
    pub cols: u16,
    pub rows: u16,
    pub kind: MediaKind,
}

impl MediaSlot {
    pub fn new(url: String, cols: u16, rows: u16, kind: MediaKind) -> Self {
        Self {
            key: format!("{url}@{cols}x{rows}"),
            url,
            cols,
            rows,
            kind,
        }
    }
}

impl std::fmt::Debug for CustomEmojiState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            CustomEmojiState::Loading => "Loading",
            CustomEmojiState::Ready(_) => "Ready(..)",
            CustomEmojiState::Failed => "Failed",
        })
    }
}

#[derive(Debug, Clone)]
pub struct ReplyState {
    pub channel_id: String,
    pub message_id: String,
    pub author_name: String,
    pub source_guild_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EditState {
    pub channel_id: String,
    pub message_id: String,
}

#[derive(Debug, Clone)]
pub struct PickerEntry {
    pub server: ServerSelection,
    pub channel_id: String,
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct ChannelPicker {
    pub query: String,
    pub entries: Vec<PickerEntry>,
    pub filtered: Vec<usize>,
    pub selected: usize,
}

#[derive(Debug, Clone)]
pub struct ReadState {
    pub last_message_id: Option<String>,
    pub mention_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationVisibility {
    AllMessages,
    MentionsOnly,
    None,
}

pub enum ImagePreviewState {
    Loading {
        title: String,
    },
    ReadyBitmap {
        title: String,
        protocol: StatefulProtocol,
    },
    ReadyAnimatedGif {
        title: String,
        frames: Vec<DynamicImage>,
        delays: Vec<Duration>,
        frame_idx: usize,
        elapsed: Duration,
        current_protocol: StatefulProtocol,
    },
    ReadyChafa {
        title: String,
        lines: Vec<String>,
        scroll: usize,
    },
    /// Console mode: the frames are blitted by our own renderer.
    ReadyPixels {
        title: String,
        frames: Vec<std::sync::Arc<image::RgbaImage>>,
        delays: Vec<Duration>,
        frame_idx: usize,
        elapsed: Duration,
    },
    Failed {
        message: String,
    },
}

impl std::fmt::Debug for ImagePreviewState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ImagePreviewState::Loading { .. } => "Loading",
            ImagePreviewState::ReadyBitmap { .. } => "ReadyBitmap",
            ImagePreviewState::ReadyAnimatedGif { .. } => "ReadyAnimatedGif",
            ImagePreviewState::ReadyChafa { .. } => "ReadyChafa",
            ImagePreviewState::ReadyPixels { .. } => "ReadyPixels",
            ImagePreviewState::Failed { .. } => "Failed",
        })
    }
}

#[derive(Debug)]
pub struct App {
    pub discovery: WellKnownFluxerResponse,
    pub me: UserPrivateResponse,
    pub user_settings: Option<UserSettingsResponse>,
    pub user_guild_settings: HashMap<Snowflake, UserGuildSettingsResponse>,
    pub guilds: Vec<GuildResponse>,
    pub private_channels: Vec<ChannelResponse>,
    pub guild_channels: HashMap<Snowflake, Vec<ChannelResponse>>,
    pub guild_members: HashMap<Snowflake, Vec<GuildMemberResponse>>,
    pub messages: HashMap<Snowflake, Vec<MessageResponse>>,
    pub user_cache: HashMap<Snowflake, UserPartialResponse>,
    pub voice_states: HashMap<Snowflake, HashMap<Snowflake, VoiceStateResponse>>,
    pub guild_emojis: HashMap<Snowflake, Vec<crate::api::types::GuildEmojiResponse>>,
    pub guild_roles: HashMap<Snowflake, Vec<crate::api::types::GuildRoleResponse>>,
    pub emoji_autocomplete: Option<EmojiAutocomplete>,
    pub mention_autocomplete: Option<MentionAutocomplete>,
    pub command_autocomplete: Option<CommandAutocomplete>,
    pub selected_server: ServerSelection,
    pub selected_channel_id: Option<String>,
    pub focus: Focus,
    pub input: String,
    /// Files staged with Ctrl+V or /attach, uploaded with the next message.
    pub pending_attachments: Vec<crate::media::StagedAttachment>,
    pub message_scroll_from_bottom: u16,
    pub message_scroll_max: u16,
    pub selected_message_index: Option<usize>,
    pub reply_to: Option<ReplyState>,
    pub forward_mode: bool,
    pub read_states: HashMap<Snowflake, ReadState>,
    pub typing_users: HashMap<Snowflake, HashMap<Snowflake, Instant>>,
    pub gateway_status: GatewayStatus,
    pub gateway_lazy_guild_id: Option<String>,
    pub status_message: String,
    status_message_until: Option<Instant>,
    pub should_quit: bool,
    pub should_logout: bool,
    pub loading_channels: HashSet<String>,
    pub loading_members: HashSet<String>,
    pub guild_members_synced: HashSet<String>,
    pub api_backoff_until: HashMap<String, Instant>,
    pub loading_messages: HashSet<String>,
    pub loading_emojis: HashSet<String>,
    pub loading_roles: HashSet<String>,
    pub guild_roles_forbidden: HashSet<String>,
    pub messages_older_exhausted: HashSet<String>,
    pub loading_older_messages: HashSet<String>,
    pub show_help: bool,
    pub help_scroll: u16,
    pub channel_picker: Option<ChannelPicker>,
    pub reaction_target: Option<(String, String)>,
    pub edit_target: Option<EditState>,
    pub input_bar_anim_phase: u8,
    pub input_bar_anim_slow: u8,
    pub image_preview: Option<ImagePreviewState>,
    pub chafa_viewport: (u16, u16),
    pub chafa_preview_cells: (u16, u16),
    pub image_picker: Option<Picker>,
    /// Console mode: pictures are blitted by our own renderer instead of a
    /// terminal graphics protocol.
    pub pixel_mode: bool,
    /// Console mode: where this frame's pictures go (shared with the backend).
    pub pixel_placements: crate::console::backend::SharedPlacements,
    /// Custom (guild) emoji images by id, encoded for the terminal's protocol.
    pub custom_emojis: HashMap<String, CustomEmojiState>,
    /// Per frame: which emoji id each marker slot in the message pane refers to.
    pub custom_emoji_slots: RefCell<Vec<String>>,
    /// Per frame: (id, animated) the renderer met but has no image for yet.
    pub custom_emoji_wanted: RefCell<Vec<(String, bool)>>,
    /// Pictures under messages and avatars, ready to draw, within a byte budget.
    pub media: crate::media::MediaCache,
    /// Per frame: the block behind each media marker slot.
    pub media_slots: RefCell<Vec<MediaSlot>>,
    /// Per frame: blocks on screen with nothing loaded for them yet.
    pub media_wanted: RefCell<Vec<MediaSlot>>,
    /// Per frame: whether an animated block was drawn (so the next tick redraws).
    pub media_animation_seen: std::cell::Cell<bool>,
    /// Pixel size of one cell: the terminal's font size or the console glyph size.
    pub cell_px: (u32, u32),
    /// Downloaded pictures kept between runs.
    pub disk_cache: Option<std::sync::Arc<crate::media::DiskCache>>,
    /// Per frame: the escape sequences to print at each picture's sentinel
    /// cell (shared with the terminal backend).
    pub terminal_pictures: crate::console::backend::SharedPictures,
    /// Per frame: the rendered cells and the pane's scroll, for the
    /// terminal backend to reconcile against what the terminal shows.
    pub terminal_frame: crate::console::backend::SharedFrame,
    /// The message pane's lines as last built, reused while nothing that
    /// shows in them changed.
    pub pane_layout: RefCell<Option<std::rc::Rc<crate::ui::message_pane::PaneLayout>>>,
    /// Bumped when names, nicknames, roles or members change.
    pub roster_version: u64,
    /// Bumped when a custom emoji's picture arrives or fails.
    pub custom_emoji_version: u64,
    /// The message pane as last drawn, to tell a plain scroll from a change.
    pub pane_last: Option<PaneView>,
    /// Where the reader is while scrolled up, kept still as content changes.
    pub pane_anchor: Option<PaneAnchor>,
    /// Set by the message pane when it merely scrolled since the last draw.
    pub pane_scroll_hint: Option<crate::console::backend::RegionScroll>,
    /// Per frame: the shortest frame delay of an animation drawn, which
    /// sets the pace of the ticks.
    pub animation_delay_seen: std::cell::Cell<Option<Duration>>,
    /// Counts UI draws; animated emoji decide their frame once per draw.
    pub draw_serial: std::cell::Cell<u64>,
    pub show_settings: bool,
    pub settings_cursor: usize,
    pub show_server_notifications: bool,
    pub server_notification_cursor: usize,
    pub server_notification_scroll: u16,
    pub ui_settings: UiSettings,
}

impl App {
    const DM_SETTINGS_KEY: &'static str = "@me";
    const SERVER_MUTE_PRESET_MS: [u64; 5] = [
        15 * 60 * 1000,
        60 * 60 * 1000,
        3 * 60 * 60 * 1000,
        8 * 60 * 60 * 1000,
        24 * 60 * 60 * 1000,
    ];

    pub fn new(
        discovery: WellKnownFluxerResponse,
        me: UserPrivateResponse,
        user_settings: Option<UserSettingsResponse>,
        guilds: Vec<GuildResponse>,
        private_channels: Vec<ChannelResponse>,
        selected_server: ServerSelection,
        selected_channel_id: Option<String>,
        ui_settings: UiSettings,
    ) -> Self {
        let mut user_cache = HashMap::new();
        merge_user_cache(
            &mut user_cache,
            private_channels
                .iter()
                .flat_map(|channel| channel.recipients.clone()),
        );

        let mut app = Self {
            discovery,
            me,
            user_settings,
            user_guild_settings: HashMap::new(),
            guilds,
            private_channels,
            guild_channels: HashMap::new(),
            guild_members: HashMap::new(),
            messages: HashMap::new(),
            user_cache,
            voice_states: HashMap::new(),
            guild_emojis: HashMap::new(),
            guild_roles: HashMap::new(),
            emoji_autocomplete: None,
            mention_autocomplete: None,
            command_autocomplete: None,
            selected_server,
            selected_channel_id,
            focus: Focus::Channels,
            input: String::new(),
            pending_attachments: Vec::new(),
            message_scroll_from_bottom: 0,
            message_scroll_max: 0,
            selected_message_index: None,
            reply_to: None,
            forward_mode: false,
            read_states: HashMap::new(),
            typing_users: HashMap::new(),
            gateway_status: GatewayStatus::Disconnected,
            gateway_lazy_guild_id: None,
            status_message: String::new(),
            status_message_until: None,
            should_quit: false,
            should_logout: false,
            loading_channels: HashSet::new(),
            loading_members: HashSet::new(),
            guild_members_synced: HashSet::new(),
            api_backoff_until: HashMap::new(),
            loading_messages: HashSet::new(),
            loading_emojis: HashSet::new(),
            loading_roles: HashSet::new(),
            guild_roles_forbidden: HashSet::new(),
            messages_older_exhausted: HashSet::new(),
            loading_older_messages: HashSet::new(),
            show_help: false,
            help_scroll: 0,
            channel_picker: None,
            reaction_target: None,
            edit_target: None,
            input_bar_anim_phase: 0,
            input_bar_anim_slow: 0,
            image_preview: None,
            chafa_viewport: (80, 22),
            chafa_preview_cells: (100, 40),
            image_picker: None,
            pixel_mode: false,
            pixel_placements: std::rc::Rc::new(RefCell::new(Vec::new())),
            custom_emojis: HashMap::new(),
            custom_emoji_slots: RefCell::new(Vec::new()),
            custom_emoji_wanted: RefCell::new(Vec::new()),
            media: crate::media::MediaCache::new(64 << 20),
            media_slots: RefCell::new(Vec::new()),
            media_wanted: RefCell::new(Vec::new()),
            media_animation_seen: std::cell::Cell::new(false),
            cell_px: (8, 16),
            disk_cache: None,
            terminal_pictures: std::rc::Rc::new(RefCell::new(HashMap::new())),
            terminal_frame: std::rc::Rc::new(RefCell::new(Default::default())),
            pane_layout: RefCell::new(None),
            roster_version: 0,
            custom_emoji_version: 0,
            pane_last: None,
            pane_anchor: None,
            pane_scroll_hint: None,
            animation_delay_seen: std::cell::Cell::new(None),
            draw_serial: std::cell::Cell::new(0),
            show_settings: false,
            settings_cursor: 0,
            show_server_notifications: false,
            server_notification_cursor: 0,
            server_notification_scroll: 0,
            ui_settings,
        };
        app.normalize_selection();
        app
    }

    pub const UI_SETTINGS_LAST_ROW: usize = 5;
    pub const SERVER_NOTIFICATION_LAST_ROW: usize = 5;
    pub const HISTORY_AUTOLOAD_THRESHOLD_ROWS: u16 = 3;
    pub const TRANSIENT_STATUS_DURATION: Duration = Duration::from_millis(1800);

    fn user_guild_settings_key(guild_id: Option<&str>) -> String {
        guild_id.unwrap_or(Self::DM_SETTINGS_KEY).to_string()
    }

    fn default_user_guild_settings(guild_id: Option<&str>) -> UserGuildSettingsResponse {
        UserGuildSettingsResponse {
            guild_id: guild_id.map(str::to_string),
            message_notifications: if guild_id.is_some() {
                MESSAGE_NOTIFICATIONS_INHERIT
            } else {
                MESSAGE_NOTIFICATIONS_ALL_MESSAGES
            },
            muted: false,
            mute_config: None,
            mobile_push: guild_id.is_some(),
            suppress_everyone: false,
            suppress_roles: false,
            hide_muted_channels: false,
            channel_overrides: HashMap::new(),
            version: 0,
        }
    }

    fn mute_active(muted: bool, mute_config: Option<&UserGuildMuteConfig>) -> bool {
        if !muted {
            return false;
        }
        let Some(end_time) = mute_config.and_then(|config| config.end_time.as_deref()) else {
            return true;
        };
        chrono::DateTime::parse_from_rfc3339(end_time)
            .map(|deadline| deadline.with_timezone(&chrono::Utc) > chrono::Utc::now())
            .unwrap_or(true)
    }

    fn sanitize_user_guild_settings(
        mut settings: UserGuildSettingsResponse,
    ) -> UserGuildSettingsResponse {
        if !Self::mute_active(settings.muted, settings.mute_config.as_ref()) {
            settings.muted = false;
            settings.mute_config = None;
        }
        settings.channel_overrides.retain(|_, override_settings| {
            if Self::mute_active(
                override_settings.muted,
                override_settings.mute_config.as_ref(),
            ) {
                true
            } else {
                override_settings.muted = false;
                override_settings.mute_config = None;
                true
            }
        });
        settings
    }

    pub fn selected_server_name(&self) -> String {
        match &self.selected_server {
            ServerSelection::DirectMessages => "Direct Messages".to_string(),
            ServerSelection::Guild(id) => self
                .guilds
                .iter()
                .find(|guild| guild.id == *id)
                .map(|guild| guild.name.clone())
                .unwrap_or_else(|| id.clone()),
        }
    }

    pub fn selected_server_guild_id(&self) -> Option<String> {
        match &self.selected_server {
            ServerSelection::DirectMessages => None,
            ServerSelection::Guild(id) => Some(id.clone()),
        }
    }

    fn user_guild_settings_for(&self, guild_id: Option<&str>) -> UserGuildSettingsResponse {
        self.user_guild_settings
            .get(Self::user_guild_settings_key(guild_id).as_str())
            .cloned()
            .map(Self::sanitize_user_guild_settings)
            .unwrap_or_else(|| Self::default_user_guild_settings(guild_id))
    }

    fn user_guild_settings_mut_or_default(
        &mut self,
        guild_id: Option<&str>,
    ) -> &mut UserGuildSettingsResponse {
        let key = Self::user_guild_settings_key(guild_id);
        self.user_guild_settings
            .entry(key)
            .or_insert_with(|| Self::default_user_guild_settings(guild_id))
    }

    pub fn selected_server_notification_settings(&self) -> Option<UserGuildSettingsResponse> {
        let guild_id = self.selected_server_guild_id()?;
        Some(self.user_guild_settings_for(Some(&guild_id)))
    }

    pub fn set_user_guild_settings(&mut self, settings: Vec<UserGuildSettingsResponse>) {
        self.user_guild_settings.clear();
        for settings_entry in settings {
            self.upsert_user_guild_settings(settings_entry);
        }
    }

    pub fn upsert_user_guild_settings(&mut self, settings: UserGuildSettingsResponse) {
        let key = Self::user_guild_settings_key(settings.guild_id.as_deref());
        self.user_guild_settings
            .insert(key, Self::sanitize_user_guild_settings(settings));
        self.normalize_selection();
    }

    pub fn apply_user_guild_settings_patch(
        &mut self,
        guild_id: Option<&str>,
        patch: &UserGuildSettingsPatch,
    ) {
        let settings = self.user_guild_settings_mut_or_default(guild_id);
        if let Some(value) = patch.message_notifications {
            settings.message_notifications = value;
        }
        if let Some(value) = patch.muted {
            settings.muted = value;
        }
        if let Some(value) = &patch.mute_config {
            settings.mute_config = value.clone();
        }
        if let Some(value) = patch.mobile_push {
            settings.mobile_push = value;
        }
        if let Some(value) = patch.suppress_everyone {
            settings.suppress_everyone = value;
        }
        if let Some(value) = patch.suppress_roles {
            settings.suppress_roles = value;
        }
        if let Some(value) = patch.hide_muted_channels {
            settings.hide_muted_channels = value;
        }

        let sanitized = Self::sanitize_user_guild_settings(settings.clone());
        *settings = sanitized;
        self.normalize_selection();
    }

    pub fn open_server_notification_settings(&mut self) -> bool {
        let Some(_) = self.selected_server_guild_id() else {
            self.set_status("Select a community to edit its notification settings.");
            return false;
        };
        self.dismiss_image_preview();
        self.show_server_notifications = true;
        self.server_notification_cursor = 0;
        self.server_notification_scroll = 0;
        true
    }

    pub fn current_server_mute_choice_index(&self) -> Option<usize> {
        let settings = self.selected_server_notification_settings()?;
        if !Self::mute_active(settings.muted, settings.mute_config.as_ref()) {
            return Some(0);
        }
        let preset = settings
            .mute_config
            .as_ref()
            .and_then(|config| config.selected_time_window);
        if let Some(window) = preset
            && let Some(index) = Self::SERVER_MUTE_PRESET_MS
                .iter()
                .position(|candidate| *candidate == window)
        {
            return Some(index + 1);
        }
        Some(Self::SERVER_MUTE_PRESET_MS.len() + 1)
    }

    pub fn cycle_server_notification_setting(
        &mut self,
        delta: i32,
    ) -> Option<(String, UserGuildSettingsPatch)> {
        let guild_id = self.selected_server_guild_id()?;
        let settings = self.user_guild_settings_for(Some(&guild_id));

        let patch = match self.server_notification_cursor {
            0 => {
                let option_count = Self::SERVER_MUTE_PRESET_MS.len() as i32 + 2;
                let current = self.current_server_mute_choice_index()? as i32;
                let next = (current + delta).rem_euclid(option_count) as usize;
                if next == 0 {
                    UserGuildSettingsPatch {
                        muted: Some(false),
                        mute_config: Some(None),
                        ..UserGuildSettingsPatch::default()
                    }
                } else if next == Self::SERVER_MUTE_PRESET_MS.len() + 1 {
                    UserGuildSettingsPatch {
                        muted: Some(true),
                        mute_config: Some(None),
                        ..UserGuildSettingsPatch::default()
                    }
                } else {
                    let window = Self::SERVER_MUTE_PRESET_MS[next - 1];
                    UserGuildSettingsPatch {
                        muted: Some(true),
                        mute_config: Some(Some(UserGuildMuteConfig {
                            end_time: Some(
                                (chrono::Utc::now()
                                    + chrono::Duration::milliseconds(window as i64))
                                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                            ),
                            selected_time_window: Some(window),
                        })),
                        ..UserGuildSettingsPatch::default()
                    }
                }
            }
            1 => {
                let options = [
                    MESSAGE_NOTIFICATIONS_ALL_MESSAGES,
                    MESSAGE_NOTIFICATIONS_ONLY_MENTIONS,
                    MESSAGE_NOTIFICATIONS_NO_MESSAGES,
                ];
                let current = self.resolved_message_notifications_for_guild(&guild_id);
                let current_index = options
                    .iter()
                    .position(|candidate| *candidate == current)
                    .unwrap_or(0) as i32;
                let next = (current_index + delta).rem_euclid(options.len() as i32) as usize;
                UserGuildSettingsPatch {
                    message_notifications: Some(options[next]),
                    ..UserGuildSettingsPatch::default()
                }
            }
            2 => UserGuildSettingsPatch {
                suppress_everyone: Some(!settings.suppress_everyone),
                ..UserGuildSettingsPatch::default()
            },
            3 => UserGuildSettingsPatch {
                suppress_roles: Some(!settings.suppress_roles),
                ..UserGuildSettingsPatch::default()
            },
            4 => UserGuildSettingsPatch {
                hide_muted_channels: Some(!settings.hide_muted_channels),
                ..UserGuildSettingsPatch::default()
            },
            5 => UserGuildSettingsPatch {
                mobile_push: Some(!settings.mobile_push),
                ..UserGuildSettingsPatch::default()
            },
            _ => return None,
        };

        self.apply_user_guild_settings_patch(Some(&guild_id), &patch);
        Some((guild_id, patch))
    }

    pub fn toggle_settings_selection(&mut self) {
        match self.settings_cursor {
            0 => {
                self.ui_settings.clock_12h = !self.ui_settings.clock_12h;
            }
            1 => {
                self.ui_settings.show_typing_indicators = !self.ui_settings.show_typing_indicators;
            }
            2 => {
                self.ui_settings.performance_mode = !self.ui_settings.performance_mode;
            }
            3 => {
                use crate::config::Theme;
                self.ui_settings.theme = match self.ui_settings.theme {
                    Theme::Terminal => Theme::Fluxer,
                    Theme::Fluxer => Theme::Terminal,
                };
                crate::ui::theme::set_terminal_theme(self.ui_settings.theme == Theme::Terminal);
            }
            4 => {
                self.ui_settings.inline_media = !self.ui_settings.inline_media;
            }
            5 => {
                self.ui_settings.avatars = !self.ui_settings.avatars;
            }
            _ => {}
        }
    }

    pub fn suppress_everyone_enabled(&self, guild_id: Option<&str>) -> bool {
        guild_id
            .map(|id| self.user_guild_settings_for(Some(id)).suppress_everyone)
            .unwrap_or(false)
    }

    pub fn suppress_roles_enabled(&self, guild_id: Option<&str>) -> bool {
        guild_id
            .map(|id| self.user_guild_settings_for(Some(id)).suppress_roles)
            .unwrap_or(false)
    }

    pub fn hide_muted_channels_enabled(&self, guild_id: &str) -> bool {
        self.user_guild_settings_for(Some(guild_id))
            .hide_muted_channels
    }

    pub fn guild_is_muted(&self, guild_id: Option<&str>) -> bool {
        let settings = self.user_guild_settings_for(guild_id);
        Self::mute_active(settings.muted, settings.mute_config.as_ref())
    }

    pub fn channel_override(
        &self,
        guild_id: Option<&str>,
        channel_id: &str,
    ) -> Option<UserGuildChannelOverride> {
        self.user_guild_settings_for(guild_id)
            .channel_overrides
            .get(channel_id)
            .cloned()
    }

    pub fn channel_is_muted_directly(&self, channel: &ChannelResponse) -> bool {
        self.channel_override(channel.guild_id.as_deref(), &channel.id)
            .is_some_and(|override_settings| {
                Self::mute_active(
                    override_settings.muted,
                    override_settings.mute_config.as_ref(),
                )
            })
    }

    pub fn channel_parent_is_muted(&self, channel: &ChannelResponse) -> bool {
        let Some(parent_id) = channel.parent_id.as_deref() else {
            return false;
        };
        self.channel_override(channel.guild_id.as_deref(), parent_id)
            .is_some_and(|override_settings| {
                Self::mute_active(
                    override_settings.muted,
                    override_settings.mute_config.as_ref(),
                )
            })
    }

    pub fn channel_is_muted_effective(&self, channel: &ChannelResponse) -> bool {
        self.guild_is_muted(channel.guild_id.as_deref())
            || self.channel_parent_is_muted(channel)
            || self.channel_is_muted_directly(channel)
    }

    pub fn resolved_message_notifications_for_guild(&self, guild_id: &str) -> i32 {
        let settings = self.user_guild_settings_for(Some(guild_id));
        if settings.message_notifications != MESSAGE_NOTIFICATIONS_INHERIT {
            return settings.message_notifications;
        }
        self.guilds
            .iter()
            .find(|guild| guild.id == guild_id)
            .map(|guild| guild.default_message_notifications)
            .unwrap_or(MESSAGE_NOTIFICATIONS_ALL_MESSAGES)
    }

    pub fn resolved_message_notifications(&self, channel: &ChannelResponse) -> i32 {
        let guild_id = channel.guild_id.as_deref();
        let Some(guild_id) = guild_id else {
            return self
                .channel_override(None, &channel.id)
                .map(|override_settings| override_settings.message_notifications)
                .filter(|level| *level != MESSAGE_NOTIFICATIONS_INHERIT)
                .unwrap_or(MESSAGE_NOTIFICATIONS_ALL_MESSAGES);
        };

        if let Some(override_settings) = self.channel_override(Some(guild_id), &channel.id)
            && override_settings.message_notifications != MESSAGE_NOTIFICATIONS_INHERIT
        {
            return override_settings.message_notifications;
        }

        if let Some(parent_id) = channel.parent_id.as_deref()
            && let Some(parent_override) = self.channel_override(Some(guild_id), parent_id)
            && parent_override.message_notifications != MESSAGE_NOTIFICATIONS_INHERIT
        {
            return parent_override.message_notifications;
        }

        self.resolved_message_notifications_for_guild(guild_id)
    }

    pub fn channel_notification_visibility(
        &self,
        channel: &ChannelResponse,
    ) -> NotificationVisibility {
        let level = self.resolved_message_notifications(channel);
        if level == MESSAGE_NOTIFICATIONS_NO_MESSAGES {
            return NotificationVisibility::None;
        }
        if self.channel_is_muted_effective(channel) || level == MESSAGE_NOTIFICATIONS_ONLY_MENTIONS
        {
            return NotificationVisibility::MentionsOnly;
        }
        NotificationVisibility::AllMessages
    }

    pub fn visible_channel_is_unread(&self, channel_id: &str) -> bool {
        let Some(channel) = self.channel_by_id(channel_id) else {
            return false;
        };
        self.channel_notification_visibility(&channel) == NotificationVisibility::AllMessages
            && self.channel_is_unread(channel_id)
    }

    pub fn visible_channel_mention_count(&self, channel_id: &str) -> u64 {
        let Some(channel) = self.channel_by_id(channel_id) else {
            return 0;
        };
        match self.channel_notification_visibility(&channel) {
            NotificationVisibility::None => 0,
            NotificationVisibility::AllMessages | NotificationVisibility::MentionsOnly => {
                self.channel_mention_count(channel_id)
            }
        }
    }

    fn channel_hidden_in_sidebar(&self, channel: &ChannelResponse) -> bool {
        let Some(guild_id) = channel.guild_id.as_deref() else {
            return false;
        };
        if !self.hide_muted_channels_enabled(guild_id) {
            return false;
        }
        if self.selected_channel_id.as_deref() == Some(channel.id.as_str()) {
            return false;
        }
        self.channel_is_muted_effective(channel)
    }

    pub const API_FAILURE_BACKOFF_SECS: u64 = 180;

    pub fn api_backoff_can_try(&self, key: &str) -> bool {
        self.api_backoff_until
            .get(key)
            .is_none_or(|until| Instant::now() >= *until)
    }

    pub fn api_backoff_after_failure(&mut self, key: impl Into<String>) {
        self.api_backoff_until.insert(
            key.into(),
            Instant::now() + Duration::from_secs(Self::API_FAILURE_BACKOFF_SECS),
        );
    }

    pub fn api_backoff_clear(&mut self, key: &str) {
        self.api_backoff_until.remove(key);
    }

    pub fn api_backoff_clear_guild(&mut self, guild_id: &str) {
        for prefix in ["members:", "channels:", "emojis:", "roles:"] {
            self.api_backoff_until
                .remove(&format!("{prefix}{guild_id}"));
        }
    }

    pub fn api_backoff_clear_channel_messages(&mut self, channel_id: &str) {
        self.api_backoff_until
            .remove(&format!("messages:{channel_id}"));
    }

    pub fn set_guild_emojis(
        &mut self,
        guild_id: &str,
        emojis: Vec<crate::api::types::GuildEmojiResponse>,
    ) {
        self.guild_emojis.insert(guild_id.to_string(), emojis);
        self.loading_emojis.remove(guild_id);
        self.api_backoff_clear(&format!("emojis:{guild_id}"));
    }

    pub fn set_guild_roles(
        &mut self,
        guild_id: &str,
        roles: Vec<crate::api::types::GuildRoleResponse>,
    ) {
        self.roster_version = self.roster_version.wrapping_add(1);
        self.guild_roles.insert(guild_id.to_string(), roles);
        self.loading_roles.remove(guild_id);
        self.api_backoff_clear(&format!("roles:{guild_id}"));
    }

    pub fn merge_guild_roles_from_gateway(
        &mut self,
        guild_id: &str,
        incoming: Vec<crate::api::types::GuildRoleResponse>,
    ) {
        self.roster_version = self.roster_version.wrapping_add(1);
        if incoming.is_empty() {
            return;
        }
        let entry = self.guild_roles.entry(guild_id.to_string()).or_default();
        for r in incoming {
            if r.id.is_empty() {
                continue;
            }
            let id_trim = r.id.trim().to_string();
            if let Some(existing) = entry.iter_mut().find(|e| e.id.trim() == id_trim.as_str()) {
                *existing = r;
            } else {
                entry.push(r);
            }
        }
    }

    pub fn remove_guild_role(&mut self, guild_id: &str, role_id: &str) {
        self.roster_version = self.roster_version.wrapping_add(1);
        let Some(roles) = self.guild_roles.get_mut(guild_id) else {
            return;
        };
        let rid = role_id.trim();
        roles.retain(|r| r.id.trim() != rid);
    }

    pub fn server_entries(&self) -> Vec<ServerSelection> {
        let mut entries = vec![ServerSelection::DirectMessages];
        entries.extend(
            self.guilds
                .iter()
                .map(|guild| ServerSelection::Guild(guild.id.clone())),
        );
        entries
    }

    pub fn server_selected_index(&self) -> usize {
        self.server_entries()
            .iter()
            .position(|entry| entry == &self.selected_server)
            .unwrap_or_default()
    }

    pub fn move_server(&mut self, delta: i32) -> bool {
        let entries = self.server_entries();
        if entries.is_empty() {
            return false;
        }
        let current = self.server_selected_index() as i32;
        let next = (current + delta).clamp(0, entries.len() as i32 - 1) as usize;
        if entries[next] == self.selected_server {
            return false;
        }
        self.selected_server = entries[next].clone();
        self.selected_channel_id = None;
        self.message_scroll_from_bottom = 0;
        self.normalize_selection();
        true
    }

    pub fn all_channels_for_server(&self, server: &ServerSelection) -> Vec<ChannelResponse> {
        match server {
            ServerSelection::DirectMessages => {
                let mut dms = self.private_channels.clone();
                dms.sort_by(|a, b| {
                    let a_key = a
                        .last_message_id
                        .as_deref()
                        .and_then(|id| id.parse::<u128>().ok())
                        .unwrap_or(0);
                    let b_key = b
                        .last_message_id
                        .as_deref()
                        .and_then(|id| id.parse::<u128>().ok())
                        .unwrap_or(0);
                    b_key.cmp(&a_key)
                });
                dms
            }
            ServerSelection::Guild(guild_id) => {
                let all = self
                    .guild_channels
                    .get(guild_id)
                    .cloned()
                    .unwrap_or_default();

                let mut categories: Vec<&ChannelResponse> = all
                    .iter()
                    .filter(|c| c.channel_type() == CHANNEL_GUILD_CATEGORY)
                    .collect();
                categories.sort_by_key(|c| c.position);

                let mut non_cat: Vec<&ChannelResponse> = all
                    .iter()
                    .filter(|c| c.channel_type() != CHANNEL_GUILD_CATEGORY)
                    .collect();
                non_cat.sort_by(|a, b| a.position.cmp(&b.position).then(a.name.cmp(&b.name)));

                let mut result: Vec<ChannelResponse> = Vec::new();

                let uncategorized: Vec<&ChannelResponse> = non_cat
                    .iter()
                    .filter(|c| c.parent_id.is_none())
                    .copied()
                    .collect();
                for ch in uncategorized {
                    result.push(ch.clone());
                }

                for cat in &categories {
                    let children: Vec<&ChannelResponse> = non_cat
                        .iter()
                        .filter(|c| c.parent_id.as_deref() == Some(cat.id.as_str()))
                        .copied()
                        .collect();
                    result.push((*cat).clone());
                    for ch in children {
                        result.push(ch.clone());
                    }
                }

                result
            }
        }
    }

    pub fn channels_for_server(&self, server: &ServerSelection) -> Vec<ChannelResponse> {
        let all = self.all_channels_for_server(server);
        let ServerSelection::Guild(guild_id) = server else {
            return all;
        };
        if !self.hide_muted_channels_enabled(guild_id) {
            return all;
        }

        let mut visible = Vec::new();
        for channel in all {
            if channel.channel_type() == CHANNEL_GUILD_CATEGORY {
                let has_visible_children = self
                    .guild_channels
                    .get(guild_id)
                    .into_iter()
                    .flat_map(|channels| channels.iter())
                    .any(|candidate| {
                        candidate.parent_id.as_deref() == Some(channel.id.as_str())
                            && !self.channel_hidden_in_sidebar(candidate)
                    });
                if has_visible_children {
                    visible.push(channel);
                }
                continue;
            }
            if !self.channel_hidden_in_sidebar(&channel) {
                visible.push(channel);
            }
        }
        visible
    }

    pub fn channel_entries(&self) -> Vec<ChannelResponse> {
        self.channels_for_server(&self.selected_server)
    }

    pub fn channel_selected_index(&self) -> usize {
        self.channel_entries()
            .iter()
            .position(|channel| Some(channel.id.as_str()) == self.selected_channel_id.as_deref())
            .unwrap_or_default()
    }

    pub fn move_channel(&mut self, delta: i32) -> bool {
        let channels = self.channel_entries();
        if channels.is_empty() {
            self.selected_channel_id = None;
            return false;
        }

        let current = self.channel_selected_index() as i32;
        let mut next = current;
        let len = channels.len() as i32;
        loop {
            next = (next + delta).clamp(0, len - 1);
            if channels[next as usize].channel_type() != CHANNEL_GUILD_CATEGORY {
                break;
            }
            if next == 0 || next == len - 1 {
                break;
            }
        }
        let next = next as usize;
        if channels[next].channel_type() == CHANNEL_GUILD_CATEGORY {
            return false;
        }
        let next_id = channels[next].id.clone();
        if self.selected_channel_id.as_deref() == Some(next_id.as_str()) {
            return false;
        }

        self.selected_channel_id = Some(next_id);
        self.message_scroll_from_bottom = 0;
        self.selected_message_index = None;
        true
    }

    pub fn move_channel_wrapping(&mut self, delta: i32) -> bool {
        let channels: Vec<ChannelResponse> = self
            .channel_entries()
            .into_iter()
            .filter(|c| c.channel_type() != CHANNEL_GUILD_CATEGORY)
            .collect();
        if channels.is_empty() {
            return false;
        }
        let current = self
            .selected_channel_id
            .as_deref()
            .and_then(|sid| channels.iter().position(|c| c.id == sid));
        let idx = match current {
            Some(i) => (i as i32 + delta).rem_euclid(channels.len() as i32) as usize,
            None => 0,
        };
        let next_id = channels[idx].id.clone();
        if self.selected_channel_id.as_deref() == Some(next_id.as_str()) {
            return false;
        }
        self.selected_channel_id = Some(next_id);
        self.message_scroll_from_bottom = 0;
        self.selected_message_index = None;
        true
    }

    pub fn navigable_channel_pairs(&self) -> Vec<(ServerSelection, String)> {
        let mut out = Vec::new();
        for server in self.server_entries() {
            for ch in self.all_channels_for_server(&server) {
                if ch.channel_type() == CHANNEL_GUILD_CATEGORY {
                    continue;
                }
                if matches!(
                    ch.channel_type(),
                    CHANNEL_GUILD_TEXT
                        | CHANNEL_DM
                        | CHANNEL_GROUP_DM
                        | CHANNEL_DM_PERSONAL_NOTES
                        | CHANNEL_GUILD_LINK
                ) {
                    out.push((server.clone(), ch.id.clone()));
                }
            }
        }
        out
    }

    pub fn next_channel_with_activity(&self) -> Option<(ServerSelection, String)> {
        let flat = self.navigable_channel_pairs();
        if flat.len() < 2 {
            return None;
        }
        let pos = flat
            .iter()
            .position(|(s, id)| {
                s == &self.selected_server
                    && Some(id.as_str()) == self.selected_channel_id.as_deref()
            })
            .unwrap_or(0);
        for step in 1..flat.len() {
            let i = (pos + step) % flat.len();
            let (srv, cid) = &flat[i];
            if self.visible_channel_is_unread(cid) || self.visible_channel_mention_count(cid) > 0 {
                return Some((srv.clone(), cid.clone()));
            }
        }
        None
    }

    pub fn can_edit_message(&self, msg: &MessageResponse) -> bool {
        if !self.active_channel_is_text() || !self.can_send_in_active_channel() {
            return false;
        }
        msg.author.id == self.me.id
    }

    pub fn can_delete_message(&self, msg: &MessageResponse) -> bool {
        if !self.active_channel_is_text() {
            return false;
        }
        let p = self.active_channel_permissions();
        if msg.author.id == self.me.id {
            return p & crate::permissions::VIEW_CHANNEL != 0;
        }
        p & crate::permissions::MANAGE_MESSAGES != 0
    }

    pub fn start_edit_message(&mut self, msg: MessageResponse) {
        self.reply_to = None;
        self.forward_mode = false;
        self.edit_target = Some(EditState {
            channel_id: msg.channel_id.clone(),
            message_id: msg.id.clone(),
        });
        self.input = msg.content.clone();
    }

    pub fn active_channel(&self) -> Option<ChannelResponse> {
        let active_id = self.selected_channel_id.as_deref()?;
        self.channel_entries()
            .into_iter()
            .find(|channel| channel.id == active_id)
    }

    pub fn active_channel_id(&self) -> Option<String> {
        self.selected_channel_id.clone()
    }

    pub fn guild_id_for_channel(&self, channel_id: &str) -> Option<String> {
        for (guild_id, channels) in &self.guild_channels {
            if channels.iter().any(|c| c.id == channel_id) {
                return Some(guild_id.clone());
            }
        }
        None
    }

    pub fn guild_id_for_active_channel(&self) -> Option<String> {
        let cid = self.selected_channel_id.as_deref()?;
        self.guild_id_for_channel(cid)
    }

    pub fn active_guild_id(&self) -> Option<String> {
        match &self.selected_server {
            ServerSelection::DirectMessages => None,
            ServerSelection::Guild(id) => Some(id.clone()),
        }
    }

    pub fn active_channel_is_text(&self) -> bool {
        self.active_channel()
            .map(|channel| {
                matches!(
                    channel.channel_type(),
                    CHANNEL_GUILD_TEXT
                        | CHANNEL_DM
                        | CHANNEL_GROUP_DM
                        | CHANNEL_DM_PERSONAL_NOTES
                        | CHANNEL_GUILD_LINK
                )
            })
            .unwrap_or(false)
    }

    pub fn active_channel_is_voice(&self) -> bool {
        self.active_channel()
            .map(|channel| channel.channel_type() == CHANNEL_GUILD_VOICE)
            .unwrap_or(false)
    }

    pub fn active_channel_is_link(&self) -> bool {
        self.active_channel()
            .map(|channel| channel.channel_type() == CHANNEL_GUILD_LINK)
            .unwrap_or(false)
    }

    pub fn active_channel_permissions(&self) -> u64 {
        self.active_channel()
            .map(|ch| self.channel_permissions(&ch))
            .unwrap_or(u64::MAX)
    }

    pub fn channel_permissions(&self, channel: &ChannelResponse) -> u64 {
        let Some(guild_id) = channel.guild_id.as_deref() else {
            return u64::MAX;
        };

        let guild = self.guilds.iter().find(|g| g.id == guild_id);
        let guild_base = guild
            .and_then(|g| g.permissions.as_deref())
            .and_then(|p| p.parse::<u64>().ok())
            .unwrap_or(0);
        let owner_id = guild.map(|g| g.owner_id.as_str()).unwrap_or("");

        let member_roles = self
            .guild_members
            .get(guild_id)
            .and_then(|members| members.iter().find(|m| m.user.id == self.me.id))
            .map(|m| m.roles.clone())
            .unwrap_or_default();

        crate::permissions::compute_channel_permissions(
            &self.me.id,
            &member_roles,
            guild_id,
            owner_id,
            guild_base,
            &channel.permission_overwrites,
        )
    }

    fn channel_by_id(&self, channel_id: &str) -> Option<&ChannelResponse> {
        self.private_channels
            .iter()
            .find(|c| c.id == channel_id)
            .or_else(|| {
                self.guild_channels
                    .values()
                    .flat_map(|v| v.iter())
                    .find(|c| c.id == channel_id)
            })
    }

    pub fn patch_channel_last_message_id(&mut self, channel_id: &str, message_id: &str) {
        let bump = |last: &Option<String>| match last {
            None => true,
            Some(prev) => snowflake_sort_key(message_id) > snowflake_sort_key(prev),
        };
        for c in &mut self.private_channels {
            if c.id == channel_id && bump(&c.last_message_id) {
                c.last_message_id = Some(message_id.to_string());
                return;
            }
        }
        for channels in self.guild_channels.values_mut() {
            if let Some(c) = channels.iter_mut().find(|c| c.id == channel_id) {
                if bump(&c.last_message_id) {
                    c.last_message_id = Some(message_id.to_string());
                }
                return;
            }
        }
    }

    fn message_notifies_me(&self, message: &MessageResponse) -> bool {
        let Some(channel) = self.channel_by_id(&message.channel_id) else {
            return message.mentions.iter().any(|user| user.id == self.me.id);
        };
        if self.channel_notification_visibility(&channel) == NotificationVisibility::None {
            return false;
        }
        if message.mentions.iter().any(|u| u.id == self.me.id) {
            return true;
        }
        if !message.mention_roles.is_empty()
            && !self.suppress_roles_enabled(channel.guild_id.as_deref())
            && let Some(gid) = channel.guild_id.as_deref()
            && let Some(roles) = self
                .guild_members
                .get(gid)
                .and_then(|mems| mems.iter().find(|m| m.user.id == self.me.id))
                .map(|member| member.roles.as_slice())
            && message
                .mention_roles
                .iter()
                .any(|role_id| roles.contains(role_id))
        {
            return true;
        }
        if message.mention_everyone && !self.suppress_everyone_enabled(channel.guild_id.as_deref())
        {
            return true;
        }
        if channel.guild_id.is_none() && !self.channel_is_muted_effective(&channel) {
            return true;
        }
        false
    }

    pub fn on_gateway_message_create(&mut self, message: &MessageResponse) {
        self.patch_channel_last_message_id(&message.channel_id, &message.id);

        let channel_id = message.channel_id.as_str();
        let viewing_here = self.active_channel_id().as_deref() == Some(channel_id);
        let from_self = message.author.id == self.me.id;

        if viewing_here {
            self.read_states.insert(
                message.channel_id.clone(),
                ReadState {
                    last_message_id: Some(message.id.clone()),
                    mention_count: 0,
                },
            );
            return;
        }

        if from_self {
            let mc = self
                .read_states
                .get(channel_id)
                .map(|r| r.mention_count)
                .unwrap_or(0);
            self.read_states.insert(
                message.channel_id.clone(),
                ReadState {
                    last_message_id: Some(message.id.clone()),
                    mention_count: mc,
                },
            );
            return;
        }

        self.read_states
            .entry(message.channel_id.clone())
            .or_insert(ReadState {
                last_message_id: None,
                mention_count: 0,
            });

        if self.message_notifies_me(message) {
            if let Some(rs) = self.read_states.get_mut(channel_id) {
                rs.mention_count = rs.mention_count.saturating_add(1);
            }
        }
    }

    pub fn can_send_in_active_channel(&self) -> bool {
        let p = self.active_channel_permissions();
        p & crate::permissions::VIEW_CHANNEL != 0 && p & crate::permissions::SEND_MESSAGES != 0
    }

    pub fn active_messages(&self) -> Vec<MessageResponse> {
        let Some(channel_id) = self.selected_channel_id.as_deref() else {
            return Vec::new();
        };
        let mut messages = self.messages.get(channel_id).cloned().unwrap_or_default();
        messages.sort_by_key(|message| snowflake_sort_key(&message.id));
        messages
    }

    pub fn active_oldest_message_id(&self) -> Option<String> {
        let channel_id = self.selected_channel_id.as_deref()?;
        self.messages
            .get(channel_id)
            .and_then(|messages| messages.first())
            .map(|message| message.id.clone())
    }

    /// `G`: the newest message, at the bottom of the pane; in selection
    /// mode it is selected as well.
    pub fn jump_to_latest_message(&mut self) {
        self.message_scroll_from_bottom = 0;
        self.pane_anchor = None;
        if self.selected_message_index.is_some() {
            let count = self.active_messages().len();
            self.selected_message_index = count.checked_sub(1);
        }
    }

    pub fn scroll_messages_up(&mut self, amount: u16) {
        self.message_scroll_from_bottom = self.message_scroll_from_bottom.saturating_add(amount);
    }

    pub fn scroll_messages_down(&mut self, amount: u16) {
        self.message_scroll_from_bottom = self.message_scroll_from_bottom.saturating_sub(amount);
    }

    /// Whether the terminal can draw pictures inline (sixel, kitty, iTerm2);
    /// half-block "pictures" two cells wide are not worth it.
    pub fn custom_emoji_inline_supported(&self) -> bool {
        if self.pixel_mode {
            return true;
        }
        self.image_picker
            .as_ref()
            .map(|p| p.protocol_type() != ratatui_image::picker::ProtocolType::Halfblocks)
            .unwrap_or(false)
    }

    /// The media-proxy URL of a custom emoji, as the web app builds it.
    /// Animated ones are asked for with their animation, as animated WebP.
    pub fn custom_emoji_url(&self, id: &str, animated: bool) -> String {
        let base = self.media_base_url();
        if animated {
            format!("{base}/emojis/{id}.webp?size=128&animated=true")
        } else {
            format!("{base}/emojis/{id}.webp?size=128")
        }
    }

    /// The picture of a custom emoji right now: the frames' serial, the
    /// frame index and the picture.
    pub fn custom_emoji_current(&self, id: &str) -> Option<(u16, usize, &Picture)> {
        match self.custom_emojis.get(id)? {
            CustomEmojiState::Ready(frames) => {
                if frames.is_animated() && !self.ui_settings.performance_mode {
                    self.note_animation(frames);
                }
                let (i, picture) = frames.current(Instant::now(), self.draw_serial.get());
                Some((frames.serial, i, picture))
            }
            _ => None,
        }
    }

    /// Whether the last frame drew an animated custom emoji, so the next
    /// tick should redraw to advance it.
    pub fn custom_emoji_animation_visible(&self) -> bool {
        self.custom_emoji_slots.borrow().iter().any(|id| {
            matches!(self.custom_emojis.get(id), Some(CustomEmojiState::Ready(f)) if f.is_animated())
        })
    }

    /// Called by renderers for `<:name:id>`: a marked placeholder span when
    /// the picture is ready, None to fall back to `:name:` text. Unknown ids
    /// are queued for fetching.
    pub fn custom_emoji_placeholder(&self, id: &str, animated: bool) -> Option<Span<'static>> {
        self.custom_emoji_placeholder_inner(id, animated, true)
    }

    /// The compose box is measured before it is drawn; measuring must not
    /// claim overlay slots.
    fn custom_emoji_placeholder_inner(
        &self,
        id: &str,
        animated: bool,
        register: bool,
    ) -> Option<Span<'static>> {
        if !self.custom_emoji_inline_supported() || id.is_empty() {
            return None;
        }
        match self.custom_emojis.get(id) {
            Some(CustomEmojiState::Ready(_)) => {
                if !register {
                    return Some(Span::raw(CUSTOM_EMOJI_PLACEHOLDER));
                }
                let mut slots = self.custom_emoji_slots.borrow_mut();
                let k = slots.len();
                slots.push(id.to_string());
                Some(Span::styled(
                    CUSTOM_EMOJI_PLACEHOLDER,
                    custom_emoji_marker_style(k),
                ))
            }
            Some(_) => None,
            None => {
                let mut wanted = self.custom_emoji_wanted.borrow_mut();
                if !wanted.iter().any(|(w, _)| w == id) {
                    wanted.push((id.to_string(), animated));
                }
                None
            }
        }
    }

    /// The compose box as it should be displayed: custom emoji tokens show
    /// their picture (or `:name:` while it loads), everything else verbatim.
    /// One `Line` per raw line. `register` claims overlay slots for the
    /// pictures; pass false when only measuring.
    pub fn input_display(&self, register: bool) -> Vec<Line<'static>> {
        let text_style = Style::default().fg(crate::ui::theme::text());
        let name_style = Style::default().fg(crate::ui::theme::emoji_unknown());
        let mut lines = Vec::new();
        for raw_line in self.input.split('\n') {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut rest = raw_line;
            while !rest.is_empty() {
                let Some(lt) = rest.find('<') else {
                    spans.push(Span::styled(rest.to_string(), text_style));
                    break;
                };
                if lt > 0 {
                    spans.push(Span::styled(rest[..lt].to_string(), text_style));
                }
                match parse_custom_emoji_token(&rest[lt..]) {
                    Some(tok) => {
                        match self.custom_emoji_placeholder_inner(tok.id, tok.animated, register) {
                            Some(ph) => spans.push(ph),
                            None => spans.push(Span::styled(format!(":{}:", tok.name), name_style)),
                        }
                        rest = &rest[lt + tok.len..];
                    }
                    None => {
                        spans.push(Span::styled("<".to_string(), text_style));
                        rest = &rest[lt + 1..];
                    }
                }
            }
            lines.push(Line::from(spans));
        }
        lines
    }

    /// `input_display` flattened to text, for width and cursor arithmetic.
    pub fn input_display_plain(&self) -> String {
        self.input_display(false)
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Backspace: a custom emoji token at the end goes away as one unit.
    pub fn input_pop(&mut self) {
        if self.input.ends_with('>')
            && let Some(lt) = self.input.rfind('<')
            && let Some(tok) = parse_custom_emoji_token(&self.input[lt..])
            && lt + tok.len == self.input.len()
        {
            self.input.truncate(lt);
            return;
        }
        self.input.pop();
    }

    /// Ids the last frame asked for; they are marked Loading here so each is
    /// fetched once.
    pub fn take_custom_emoji_wants(&mut self) -> Vec<(String, String)> {
        let wanted = std::mem::take(&mut *self.custom_emoji_wanted.borrow_mut());
        let mut out = Vec::new();
        for (id, animated) in wanted {
            if self.custom_emojis.contains_key(&id) {
                continue;
            }
            let url = self.custom_emoji_url(&id, animated);
            self.custom_emojis
                .insert(id.clone(), CustomEmojiState::Loading);
            out.push((id, url));
        }
        out
    }

    /// Store the decoded frames of a custom emoji (one for a still image,
    /// an empty list when the fetch or decode failed).
    pub fn set_custom_emoji_frames(&mut self, id: String, frames: Vec<(DynamicImage, Duration)>) {
        self.custom_emoji_version = self.custom_emoji_version.wrapping_add(1);
        let state = match self.image_picker.as_ref() {
            _ if self.pixel_mode && !frames.is_empty() => {
                let mut encoded = Vec::with_capacity(frames.len());
                let mut delays = Vec::with_capacity(frames.len());
                for (img, delay) in frames.into_iter().take(CUSTOM_EMOJI_MAX_FRAMES) {
                    // keep them small: they are drawn one text row tall
                    let img = if img.height() > 96 {
                        img.resize(96 * 2, 96, image::imageops::FilterType::Triangle)
                    } else {
                        img
                    };
                    encoded.push(Picture::Pixels(std::sync::Arc::new(img.to_rgba8())));
                    delays.push(delay.max(Duration::from_millis(20)));
                }
                if encoded.is_empty() {
                    CustomEmojiState::Failed
                } else {
                    CustomEmojiState::Ready(PictureFrames::new(encoded, delays))
                }
            }
            Some(picker) if !frames.is_empty() => {
                let mut encoded = Vec::with_capacity(frames.len());
                let mut delays = Vec::with_capacity(frames.len());
                // exactly the cells' pixel size: anything smaller would be
                // padded with black on sixel
                let (cw, ch) = crate::media::block_px(CUSTOM_EMOJI_CELLS, 1, self.cell_px);
                let ch = if picker.protocol_type() == ratatui_image::picker::ProtocolType::Sixel {
                    crate::media::sixel_rows(ch)
                } else {
                    ch
                };
                for (img, delay) in frames.into_iter().take(CUSTOM_EMOJI_MAX_FRAMES) {
                    let img = img.resize_exact(cw, ch, image::imageops::FilterType::Triangle);
                    if let Ok(p) = picker.new_protocol(
                        img,
                        Rect::new(0, 0, CUSTOM_EMOJI_CELLS, 1),
                        ratatui_image::Resize::Fit(None),
                    ) && let Some(tp) = terminal_picture(&p)
                    {
                        encoded.push(Picture::Terminal(std::sync::Arc::new(tp)));
                        delays.push(delay.max(Duration::from_millis(20)));
                    }
                }
                if encoded.is_empty() {
                    CustomEmojiState::Failed
                } else {
                    CustomEmojiState::Ready(PictureFrames::new(encoded, delays))
                }
            }
            _ => CustomEmojiState::Failed,
        };
        self.custom_emojis.insert(id, state);
    }

    /// The media proxy the web app uses, from discovery.
    pub fn media_base_url(&self) -> String {
        let media = self.discovery.endpoints.media.trim_end_matches('/');
        if media.is_empty() {
            "https://fluxerusercontent.com".to_string()
        } else {
            media.to_string()
        }
    }

    /// The web app's static CDN (default avatars), from discovery.
    pub fn static_cdn_url(&self) -> String {
        self.discovery
            .endpoints
            .static_cdn
            .trim_end_matches('/')
            .to_string()
    }

    /// Previews under messages: wanted, and drawable on this terminal.
    pub fn inline_media_enabled(&self) -> bool {
        self.ui_settings.inline_media && self.custom_emoji_inline_supported()
    }

    /// Profile pictures beside messages: wanted, and drawable here.
    pub fn avatars_enabled(&self) -> bool {
        self.ui_settings.avatars && self.custom_emoji_inline_supported()
    }

    /// Claim a marker slot for a block of cells this draw.
    pub fn register_media_slot(&self, slot: MediaSlot) -> usize {
        let mut slots = self.media_slots.borrow_mut();
        slots.push(slot);
        slots.len() - 1
    }

    /// The avatar block of a message author: their guild avatar, their own
    /// avatar, the web app's default avatar for their id, or (without a
    /// static CDN) a disc in their colour drawn locally.
    pub fn avatar_slot(
        &self,
        guild_id: Option<&str>,
        user: &UserPartialResponse,
        member_avatar: Option<&str>,
    ) -> MediaSlot {
        let base = self.media_base_url();
        let url = match (guild_id, member_avatar, user.avatar.as_deref()) {
            (Some(_), Some(hash), _) if !hash.is_empty() => {
                crate::media::avatar_url(&base, guild_id, &user.id, hash)
            }
            (_, _, Some(hash)) if !hash.is_empty() => {
                crate::media::avatar_url(&base, None, &user.id, hash)
            }
            _ => {
                let cdn = self.static_cdn_url();
                if cdn.is_empty() {
                    crate::media::default_avatar_key(crate::media::default_avatar_color(user))
                } else {
                    crate::media::default_avatar_url(&cdn, &user.id)
                }
            }
        };
        MediaSlot::new(
            url,
            crate::media::AVATAR_COLS,
            crate::media::AVATAR_ROWS,
            MediaKind::Avatar,
        )
    }

    /// Blocks on screen with nothing loaded yet, marked loading here so
    /// each is fetched once; a few at a time, the rest on a later draw.
    pub fn take_media_wants(&mut self) -> Vec<MediaSlot> {
        let wanted = std::mem::take(&mut *self.media_wanted.borrow_mut());
        let mut out: Vec<MediaSlot> = Vec::new();
        for slot in wanted {
            if out.iter().any(|s| s.key == slot.key) {
                continue;
            }
            if self.media.start(&slot.key) {
                out.push(slot);
            }
        }
        out
    }

    /// Store what a media download produced (None: it failed).
    pub fn set_media_frames(&mut self, key: String, frames: Option<PictureFrames>, bytes: usize) {
        self.media
            .finish(key, frames, bytes, self.draw_serial.get());
    }

    /// Whether the last draw showed an animated picture, so the next tick
    /// should redraw to advance it.
    pub fn media_animation_visible(&self) -> bool {
        self.media_animation_seen.get()
    }

    /// An animation is being drawn this frame: its pace counts for the ticks.
    pub fn note_animation(&self, frames: &PictureFrames) {
        if let Some(d) = frames.min_delay() {
            let seen = self.animation_delay_seen.get();
            self.animation_delay_seen
                .set(Some(seen.map_or(d, |s| s.min(d))));
        }
    }

    /// How long until the next tick: 100 ms, or the shortest frame delay of
    /// an animation on screen, down to 50 ms, so animations play at their
    /// own pace instead of being sampled ten times a second.
    pub fn tick_period(&self) -> Duration {
        let floor = Duration::from_millis(50);
        let mut period = Duration::from_millis(100);
        if let Some(d) = self.animation_delay_seen.get() {
            period = period.min(d.max(floor));
        }
        match &self.image_preview {
            Some(ImagePreviewState::ReadyAnimatedGif { delays, .. })
            | Some(ImagePreviewState::ReadyPixels { delays, .. }) => {
                if let Some(d) = delays.iter().copied().min() {
                    period = period.min(d.max(floor));
                }
            }
            _ => {}
        }
        period
    }

    /// Short label for the input title: "2 files: a.png, b.jpg".
    pub fn attachment_summary(&self) -> String {
        let n = self.pending_attachments.len();
        let names: Vec<String> = self
            .pending_attachments
            .iter()
            .map(|a| format!("{} {}", a.filename, a.size_label()))
            .collect();
        format!(
            "{n} {}: {}",
            if n == 1 { "file" } else { "files" },
            names.join(", ")
        )
    }

    pub fn set_status(&mut self, message: impl Into<String>) {
        self.status_message = message.into();
        self.status_message_until = None;
    }

    pub fn set_transient_status(&mut self, message: impl Into<String>, duration: Duration) {
        self.status_message = message.into();
        self.status_message_until = Some(Instant::now() + duration);
    }

    pub fn clear_status(&mut self) {
        self.status_message.clear();
        self.status_message_until = None;
    }

    pub fn expire_status_if_needed(&mut self) {
        if self
            .status_message_until
            .is_some_and(|until| Instant::now() >= until)
        {
            self.clear_status();
        }
    }

    pub fn should_auto_load_history_on_scroll_up(&self) -> bool {
        self.message_scroll_max
            .saturating_sub(self.message_scroll_from_bottom.min(self.message_scroll_max))
            <= Self::HISTORY_AUTOLOAD_THRESHOLD_ROWS
    }

    pub fn open_help(&mut self) {
        self.help_scroll = 0;
        self.show_help = true;
    }

    pub fn dismiss_image_preview(&mut self) {
        self.image_preview = None;
    }

    pub fn image_preview_scroll(&mut self, delta: i32) {
        let Some(ref mut prev) = self.image_preview else {
            return;
        };
        if let ImagePreviewState::ReadyChafa { scroll, lines, .. } = prev {
            let max = lines.len().saturating_sub(1);
            let ns = (*scroll as i32 + delta).clamp(0, max as i32) as usize;
            *scroll = ns;
        }
    }

    pub fn advance_image_preview_animation(&mut self, dt: Duration) {
        let Some(ref mut prev) = self.image_preview else {
            return;
        };
        if let ImagePreviewState::ReadyPixels {
            frames,
            delays,
            frame_idx,
            elapsed,
            ..
        } = prev
        {
            if frames.len() > 1 && !delays.is_empty() {
                *elapsed += dt;
                while *elapsed >= delays[*frame_idx % delays.len()] {
                    *elapsed -= delays[*frame_idx % delays.len()];
                    *frame_idx = (*frame_idx + 1) % frames.len();
                }
            }
            return;
        }
        let ImagePreviewState::ReadyAnimatedGif {
            frames,
            delays,
            frame_idx,
            elapsed,
            current_protocol,
            ..
        } = prev
        else {
            return;
        };
        if frames.is_empty() {
            return;
        }
        *elapsed += dt;
        let old_idx = *frame_idx;
        loop {
            let lim = delays
                .get(*frame_idx)
                .copied()
                .unwrap_or(Duration::from_millis(100));
            if *elapsed < lim {
                break;
            }
            *elapsed -= lim;
            *frame_idx = (*frame_idx + 1) % frames.len();
        }
        if *frame_idx != old_idx {
            if let Some(ref picker) = self.image_picker {
                *current_protocol = picker.new_resize_protocol(frames[*frame_idx].clone());
            }
        }
    }

    pub fn start_image_preview_loading(&mut self, title: String) {
        self.image_preview = Some(ImagePreviewState::Loading { title });
    }

    pub const TYPING_TTL: Duration = Duration::from_secs(10);

    pub fn record_typing(&mut self, channel_id: &str, user_id: &str) {
        if channel_id.is_empty() || user_id.is_empty() || user_id == self.me.id {
            return;
        }
        let exp = Instant::now() + Self::TYPING_TTL;
        self.typing_users
            .entry(channel_id.to_string())
            .or_default()
            .insert(user_id.to_string(), exp);
    }

    pub fn clear_typing_for_message(&mut self, channel_id: &str, user_id: &str) {
        if let Some(map) = self.typing_users.get_mut(channel_id) {
            map.remove(user_id);
            if map.is_empty() {
                self.typing_users.remove(channel_id);
            }
        }
    }

    pub fn prune_stale_typing(&mut self) {
        let now = Instant::now();
        self.typing_users.retain(|_, users| {
            users.retain(|_, exp| *exp > now);
            !users.is_empty()
        });
    }

    pub fn clear_all_typing(&mut self) {
        self.typing_users.clear();
    }

    pub fn typing_peer_names(&self, channel_id: &str) -> Vec<String> {
        let now = Instant::now();
        let Some(users) = self.typing_users.get(channel_id) else {
            return Vec::new();
        };
        let mut ids: Vec<&String> = users
            .iter()
            .filter(|(_, exp)| **exp > now)
            .map(|(id, _)| id)
            .collect();
        ids.sort();
        let guild = self.guild_id_for_channel(channel_id);
        ids.into_iter()
            .map(|id| {
                self.user_cache
                    .get(id.as_str())
                    .map(|u| self.shown_name_for_user(guild.as_deref(), u))
                    .unwrap_or_else(|| id.clone())
            })
            .collect()
    }

    pub fn others_typing_phrase(&self) -> Option<String> {
        if !self.ui_settings.show_typing_indicators || self.ui_settings.performance_mode {
            return None;
        }
        let ch = self.active_channel_id()?;
        let names = self.typing_peer_names(&ch);
        if names.is_empty() {
            return None;
        }
        Some(fluxer_typing_phrase(&names))
    }

    pub fn others_typing_anim_active(&self) -> bool {
        if !self.ui_settings.show_typing_indicators || self.ui_settings.performance_mode {
            return false;
        }
        self.active_channel_id()
            .is_some_and(|c| !self.typing_peer_names(&c).is_empty())
    }

    pub fn normalize_selection(&mut self) {
        let available_servers = self.server_entries();
        if !available_servers.contains(&self.selected_server) {
            self.selected_server = available_servers
                .first()
                .cloned()
                .unwrap_or(ServerSelection::DirectMessages);
        }

        let channels = self.channel_entries();
        if channels.is_empty() {
            self.selected_channel_id = None;
            return;
        }

        let selected_exists = self
            .selected_channel_id
            .as_deref()
            .map(|selected| channels.iter().any(|channel| channel.id == selected))
            .unwrap_or(false);

        if !selected_exists {
            self.selected_channel_id = channels
                .iter()
                .find(|c| c.channel_type() != CHANNEL_GUILD_CATEGORY)
                .map(|channel| channel.id.clone());
            self.message_scroll_from_bottom = 0;
        }
    }

    pub fn upsert_guild(&mut self, guild: GuildResponse) {
        if guild.id.is_empty() {
            return;
        }
        if let Some(existing) = self
            .guilds
            .iter_mut()
            .find(|existing| existing.id == guild.id)
        {
            let preserved_perms = existing.permissions.clone();
            let preserved_name = existing.name.clone();
            let preserved_owner = existing.owner_id.clone();
            *existing = guild;
            if existing.permissions.is_none() {
                existing.permissions = preserved_perms;
            }
            if existing.name.is_empty() {
                existing.name = preserved_name;
            }
            if existing.owner_id.is_empty() {
                existing.owner_id = preserved_owner;
            }
        } else {
            self.guilds.push(guild);
        }
        self.normalize_selection();
    }

    pub fn remove_guild(&mut self, guild_id: &str) {
        self.roster_version = self.roster_version.wrapping_add(1);
        self.guilds.retain(|guild| guild.id != guild_id);
        self.guild_channels.remove(guild_id);
        self.guild_members.remove(guild_id);
        self.guild_members_synced.remove(guild_id);
        self.api_backoff_clear_guild(guild_id);
        self.guild_emojis.remove(guild_id);
        self.guild_roles.remove(guild_id);
        self.guild_roles_forbidden.remove(guild_id);
        self.voice_states.remove(guild_id);
        self.normalize_selection();
    }

    pub fn set_private_channels(&mut self, channels: Vec<ChannelResponse>) {
        self.roster_version = self.roster_version.wrapping_add(1);
        merge_user_cache(
            &mut self.user_cache,
            channels
                .iter()
                .flat_map(|channel| channel.recipients.clone()),
        );
        self.private_channels = channels;
        self.normalize_selection();
    }

    pub fn upsert_private_channel(&mut self, channel: ChannelResponse) {
        if let Some(existing) = self
            .private_channels
            .iter_mut()
            .find(|existing| existing.id == channel.id)
        {
            *existing = channel;
        } else {
            self.private_channels.push(channel);
        }
        self.normalize_selection();
    }

    pub fn remove_private_channel(&mut self, channel_id: &str) {
        self.private_channels
            .retain(|channel| channel.id != channel_id);
        self.normalize_selection();
    }

    pub fn set_guild_channels(&mut self, guild_id: &str, channels: Vec<ChannelResponse>) {
        self.roster_version = self.roster_version.wrapping_add(1);
        merge_user_cache(
            &mut self.user_cache,
            channels
                .iter()
                .flat_map(|channel| channel.recipients.clone()),
        );
        self.guild_channels.insert(guild_id.to_string(), channels);
        self.loading_channels.remove(guild_id);
        self.api_backoff_clear(&format!("channels:{guild_id}"));
        self.normalize_selection();
    }

    pub fn upsert_channel(&mut self, channel: ChannelResponse) {
        if let Some(guild_id) = channel.guild_id.clone() {
            let entries = self.guild_channels.entry(guild_id).or_default();
            if let Some(existing) = entries
                .iter_mut()
                .find(|existing| existing.id == channel.id)
            {
                *existing = channel;
            } else {
                entries.push(channel);
            }
        } else {
            self.upsert_private_channel(channel);
            return;
        }
        self.normalize_selection();
    }

    pub fn remove_channel(&mut self, channel: &ChannelResponse) {
        if let Some(guild_id) = channel.guild_id.as_deref() {
            if let Some(entries) = self.guild_channels.get_mut(guild_id) {
                entries.retain(|entry| entry.id != channel.id);
            }
        } else {
            self.remove_private_channel(&channel.id);
            return;
        }
        self.normalize_selection();
    }

    pub fn set_guild_members(&mut self, guild_id: &str, members: Vec<GuildMemberResponse>) {
        self.roster_version = self.roster_version.wrapping_add(1);
        merge_user_cache(
            &mut self.user_cache,
            members.iter().map(|member| member.user.clone()),
        );
        self.guild_members.insert(guild_id.to_string(), members);
        self.loading_members.remove(guild_id);
        self.api_backoff_clear(&format!("members:{guild_id}"));
    }

    pub fn ingest_gateway_guild_members(
        &mut self,
        guild_id: &str,
        members: Vec<GuildMemberResponse>,
    ) {
        if members.is_empty() {
            return;
        }
        if self.guild_members_synced.contains(guild_id) {
            for m in members {
                self.merge_guild_member(guild_id, m);
            }
        } else {
            self.set_guild_members(guild_id, members);
        }
    }

    pub fn upsert_message(&mut self, message: MessageResponse) -> bool {
        if message.channel_id.is_empty() {
            return false;
        }
        self.merge_message_embedded_members(&message);
        merge_user_cache(&mut self.user_cache, [message.author.clone()]);
        merge_user_cache(&mut self.user_cache, message.mentions.iter().cloned());

        let channel_id = message.channel_id.clone();
        let entries = self.messages.entry(channel_id).or_default();
        let was_new = if let Some(existing) = entries
            .iter_mut()
            .find(|existing| existing.id == message.id)
        {
            *existing = message;
            false
        } else {
            entries.push(message);
            true
        };
        entries.sort_by_key(|entry| snowflake_sort_key(&entry.id));
        was_new
    }

    pub fn set_channel_messages(&mut self, channel_id: &str, mut messages: Vec<MessageResponse>) {
        for message in &messages {
            self.merge_message_embedded_members(message);
            merge_user_cache(&mut self.user_cache, [message.author.clone()]);
        }
        messages.sort_by_key(|message| snowflake_sort_key(&message.id));
        const MAX_MESSAGES: usize = 500;
        if messages.len() < 50 {
            self.messages_older_exhausted.insert(channel_id.to_string());
        } else {
            self.messages_older_exhausted.remove(channel_id);
        }
        if messages.len() > MAX_MESSAGES {
            messages.drain(0..messages.len() - MAX_MESSAGES);
        }
        self.messages.insert(channel_id.to_string(), messages);
        self.loading_messages.remove(channel_id);
        self.api_backoff_clear(&format!("messages:{channel_id}"));
        self.message_scroll_from_bottom = 0;
    }

    pub fn prepend_channel_messages(&mut self, channel_id: &str, older: Vec<MessageResponse>) {
        for message in &older {
            self.merge_message_embedded_members(message);
            merge_user_cache(&mut self.user_cache, [message.author.clone()]);
        }
        let entry = self.messages.entry(channel_id.to_string()).or_default();
        for m in older {
            if !entry.iter().any(|e| e.id == m.id) {
                entry.push(m);
            }
        }
        entry.sort_by_key(|m| snowflake_sort_key(&m.id));
        self.loading_older_messages.remove(channel_id);
    }

    pub fn remove_message(&mut self, channel_id: &str, message_id: &str) {
        if let Some(messages) = self.messages.get_mut(channel_id) {
            messages.retain(|message| message.id != message_id);
        }
    }

    pub fn update_voice_state(&mut self, state: VoiceStateResponse) {
        let Some(guild_id) = state.guild_id.clone() else {
            return;
        };

        if let Some(member) = state.member.clone() {
            self.merge_guild_member(guild_id.as_str(), member);
        }

        let guild_states = self.voice_states.entry(guild_id).or_default();
        if state.channel_id.is_none() {
            guild_states.remove(&state.user_id);
        } else {
            guild_states.insert(state.user_id.clone(), state);
        }
    }

    pub fn voice_members_for_active_channel(&self) -> Vec<String> {
        let Some(channel) = self.active_channel() else {
            return Vec::new();
        };
        let Some(guild_id) = channel.guild_id else {
            return Vec::new();
        };
        let Some(states) = self.voice_states.get(&guild_id) else {
            return Vec::new();
        };

        let mut members = states
            .values()
            .filter(|state| state.channel_id.as_deref() == Some(channel.id.as_str()))
            .map(|state| {
                let name = if let Some(m) = state.member.as_ref() {
                    let u = self.user_cache.get(&m.user.id).unwrap_or(&m.user);
                    m.nick
                        .as_ref()
                        .filter(|n| !n.trim().is_empty())
                        .cloned()
                        .unwrap_or_else(|| account_display_name(u))
                } else if let Some(u) = self.user_cache.get(&state.user_id) {
                    self.shown_name_for_user(Some(guild_id.as_str()), u)
                } else {
                    state.user_id.clone()
                };

                let mut badges = Vec::new();
                if state.self_mute {
                    badges.push("self-muted");
                }
                if state.self_deaf {
                    badges.push("self-deaf");
                }
                if state.self_stream {
                    badges.push("streaming");
                }
                if state.self_video {
                    badges.push("video");
                }

                if badges.is_empty() {
                    name
                } else {
                    format!("{name} ({})", badges.join(", "))
                }
            })
            .collect::<Vec<_>>();
        members.sort();
        members
    }
    pub fn start_emoji_autocomplete(&mut self) {
        self.emoji_autocomplete = Some(EmojiAutocomplete {
            matches: Vec::new(),
            selected_index: 0,
        });
        self.update_emoji_filter();
    }

    pub fn update_emoji_filter(&mut self) {
        if self.emoji_autocomplete.is_none() {
            return;
        }

        // Typing the closing colon of a known name completes it in place,
        // like the web composer: ":eyes:" becomes the emoji and the popup closes.
        if self.reaction_target.is_none()
            && self.input.ends_with(':')
            && let Some(open) = self.input[..self.input.len() - 1].rfind(':')
        {
            let name = self.input[open + 1..self.input.len() - 1].to_string();
            // Inside an open code span the text is meant literally.
            let in_code = self.input[..open].matches('`').count() % 2 == 1;
            if !name.is_empty() && !name.contains(char::is_whitespace) && !in_code {
                if let Some(emoji) = crate::emoji::resolve(&name) {
                    self.input.truncate(open);
                    self.input.push_str(emoji);
                }
                // Unknown name: leave the text alone; the new colon may start
                // another shortcode, which reopens the popup on the next key.
                self.emoji_autocomplete = None;
                return;
            }
        }

        let query = self.input.rsplit(':').next().unwrap_or("").to_lowercase();
        let mut results: Vec<EmojiMatch> = Vec::new();

        // guild custom emojis first
        let guild_emojis: Vec<crate::api::types::GuildEmojiResponse> = self
            .guild_id_for_active_channel()
            .and_then(|gid| self.guild_emojis.get(&gid))
            .cloned()
            .unwrap_or_default();

        for e in &guild_emojis {
            if query.is_empty() || e.name.to_lowercase().contains(&query) {
                let prefix = if e.animated { "a" } else { "" };
                results.push(EmojiMatch {
                    label: format!(":{}:", e.name),
                    insert: format!("<{}:{}:{}>", prefix, e.name, e.id),
                    is_custom: true,
                    custom_id: Some(e.id.clone()),
                    custom_animated: e.animated,
                });
            }
            if results.len() >= 12 {
                break;
            }
        }

        // standard unicode emojis, by Fluxer name, best match first
        if results.len() < 12 {
            for c in crate::emoji::search(&query, 12 - results.len()) {
                results.push(EmojiMatch {
                    label: format!("{} :{}:", c.emoji, c.name),
                    insert: c.emoji.to_string(),
                    is_custom: false,
                    custom_id: None,
                    custom_animated: false,
                });
            }
        }

        let auto = self.emoji_autocomplete.as_mut().unwrap();
        auto.matches = results;
        if auto.selected_index >= auto.matches.len() {
            auto.selected_index = auto.matches.len().saturating_sub(1);
        }
        if auto.matches.is_empty() {
            self.emoji_autocomplete = None;
        }
    }

    pub fn dismiss_emoji_autocomplete(&mut self) {
        self.emoji_autocomplete = None;
    }

    pub fn autocomplete_emoji_next(&mut self) {
        if let Some(auto) = &mut self.emoji_autocomplete
            && !auto.matches.is_empty()
        {
            auto.selected_index = (auto.selected_index + 1) % auto.matches.len();
        }
    }

    pub fn autocomplete_emoji_prev(&mut self) {
        if let Some(auto) = &mut self.emoji_autocomplete
            && !auto.matches.is_empty()
        {
            auto.selected_index =
                auto.selected_index.saturating_add(auto.matches.len() - 1) % auto.matches.len();
        }
    }

    pub fn insert_selected_emoji(&mut self) -> bool {
        if let Some(auto) = &self.emoji_autocomplete
            && let Some(emoji) = auto.matches.get(auto.selected_index)
            && let Some(colon_pos) = self.input.rfind(':')
        {
            self.input.truncate(colon_pos);
            self.input.push_str(&emoji.insert);
            self.input.push(' ');
            self.emoji_autocomplete = None;
            return true;
        }
        false
    }

    // rs

    pub fn set_read_states(&mut self, states: Vec<ReadStateResponse>) {
        for s in states {
            if !s.id.is_empty() {
                self.read_states.insert(
                    s.id,
                    ReadState {
                        last_message_id: s.last_message_id,
                        mention_count: s.mention_count,
                    },
                );
            }
        }
    }

    pub fn ack_channel(&mut self, channel_id: &str) {
        let last_msg = self.channel_last_message_id(channel_id);
        if let Some(msg_id) = last_msg {
            self.read_states.insert(
                channel_id.to_string(),
                ReadState {
                    last_message_id: Some(msg_id),
                    mention_count: 0,
                },
            );
        }
    }

    pub fn channel_is_unread(&self, channel_id: &str) -> bool {
        let Some(rs) = self.read_states.get(channel_id) else {
            return false;
        };
        let channel_last = self.channel_last_message_id(channel_id);
        match (&rs.last_message_id, &channel_last) {
            (Some(read), Some(last)) => snowflake_sort_key(read) < snowflake_sort_key(last),
            (None, Some(_)) => true,
            _ => false,
        }
    }

    pub fn channel_mention_count(&self, channel_id: &str) -> u64 {
        self.read_states
            .get(channel_id)
            .map(|rs| rs.mention_count)
            .unwrap_or(0)
    }

    fn channel_counts_toward_server_unread(&self, channel: &ChannelResponse) -> bool {
        channel.channel_type() != CHANNEL_GUILD_CATEGORY
            && (channel.channel_type() != CHANNEL_GUILD_VOICE
                || self.visible_channel_mention_count(&channel.id) > 0)
    }

    pub fn server_unread_channel_count(&self, server: &ServerSelection) -> usize {
        self.all_channels_for_server(server)
            .into_iter()
            .filter(|channel| self.channel_counts_toward_server_unread(channel))
            .filter(|channel| self.visible_channel_is_unread(&channel.id))
            .count()
    }

    pub fn server_mention_count(&self, server: &ServerSelection) -> u64 {
        self.all_channels_for_server(server)
            .into_iter()
            .filter(|channel| channel.channel_type() != CHANNEL_GUILD_CATEGORY)
            .map(|channel| self.visible_channel_mention_count(&channel.id))
            .sum()
    }

    pub(crate) fn channel_last_message_id(&self, channel_id: &str) -> Option<String> {
        let cached_last = self
            .messages
            .get(channel_id)
            .and_then(|msgs| msgs.last())
            .map(|msg| msg.id.clone());
        let channel_last = self
            .private_channels
            .iter()
            .chain(self.guild_channels.values().flat_map(|v| v.iter()))
            .find(|c| c.id == channel_id)
            .and_then(|c| c.last_message_id.clone());

        match (cached_last, channel_last) {
            (Some(cached), Some(channel)) => {
                if snowflake_sort_key(&cached) >= snowflake_sort_key(&channel) {
                    Some(cached)
                } else {
                    Some(channel)
                }
            }
            (Some(cached), None) => Some(cached),
            (None, Some(channel)) => Some(channel),
            (None, None) => None,
        }
    }

    // ms

    pub fn move_selected_message(&mut self, delta: i32) {
        let count = self.active_messages().len();
        if count == 0 {
            self.selected_message_index = None;
            return;
        }
        let current = self
            .selected_message_index
            .unwrap_or(count.saturating_sub(1));
        let next = (current as i32 + delta).clamp(0, count as i32 - 1) as usize;
        self.selected_message_index = Some(next);
        self.clamp_scroll_to_selected_message();
    }

    pub fn clamp_scroll_to_selected_message(&mut self) {
        let (w, h) = self.chafa_viewport;
        if w == 0 || h == 0 {
            return;
        }
        if let Some(s) = crate::ui::message_pane::scroll_for_selected_message(
            self,
            w.max(1),
            h.max(1),
            self.message_scroll_from_bottom,
        ) {
            self.message_scroll_from_bottom = s;
        }
    }

    pub fn selected_message(&self) -> Option<MessageResponse> {
        let msgs = self.active_messages();
        self.selected_message_index
            .and_then(|i| msgs.get(i).cloned())
    }

    // r (as in reply)

    pub fn start_reply(&mut self) {
        if let Some(msg) = self.selected_message() {
            self.edit_target = None;
            let src_guild = self.guild_id_for_channel(&msg.channel_id);
            self.reply_to = Some(ReplyState {
                channel_id: msg.channel_id.clone(),
                message_id: msg.id.clone(),
                author_name: self.shown_name_for_user(src_guild.as_deref(), &msg.author),
                source_guild_id: src_guild,
            });
            self.forward_mode = false;
            self.focus = Focus::Input;
        }
    }

    pub fn cancel_reply(&mut self) {
        self.reply_to = None;
        self.forward_mode = false;
        self.edit_target = None;
    }

    pub fn open_channel_picker(&mut self) {
        let mut entries = Vec::new();
        for server in self.server_entries() {
            let server_name = match &server {
                ServerSelection::DirectMessages => "Direct messages".to_string(),
                ServerSelection::Guild(gid) => self
                    .guilds
                    .iter()
                    .find(|g| g.id == *gid)
                    .map(|g| g.name.clone())
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| {
                        let short: String = gid.chars().take(8).collect();
                        format!("guild {short}")
                    }),
            };
            for ch in self.channels_for_server(&server) {
                if ch.channel_type() == CHANNEL_GUILD_CATEGORY {
                    continue;
                }
                if !matches!(
                    ch.channel_type(),
                    CHANNEL_GUILD_TEXT
                        | CHANNEL_DM
                        | CHANNEL_GROUP_DM
                        | CHANNEL_DM_PERSONAL_NOTES
                        | CHANNEL_GUILD_LINK
                ) {
                    continue;
                }
                let ch_label = picker_channel_line(&ch);
                let label = format!("{ch_label} · {server_name}");
                entries.push(PickerEntry {
                    server: server.clone(),
                    channel_id: ch.id.clone(),
                    label,
                });
            }
        }
        let n = entries.len();
        self.channel_picker = Some(ChannelPicker {
            query: String::new(),
            entries,
            filtered: (0..n).collect(),
            selected: 0,
        });
    }

    pub fn filter_channel_picker(&mut self) {
        let Some(p) = self.channel_picker.as_mut() else {
            return;
        };
        let q = p.query.to_lowercase();
        p.filtered = p
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| q.is_empty() || e.label.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();
        if p.filtered.is_empty() {
            p.selected = 0;
        } else {
            p.selected = p.selected.min(p.filtered.len() - 1);
        }
    }

    pub fn channel_picker_prev(&mut self) {
        let Some(p) = self.channel_picker.as_mut() else {
            return;
        };
        if p.filtered.is_empty() {
            return;
        }
        p.selected = (p.selected + p.filtered.len() - 1) % p.filtered.len();
    }

    pub fn channel_picker_next(&mut self) {
        let Some(p) = self.channel_picker.as_mut() else {
            return;
        };
        if p.filtered.is_empty() {
            return;
        }
        p.selected = (p.selected + 1) % p.filtered.len();
    }

    pub fn channel_picker_confirm(&mut self) -> bool {
        let Some(p) = &self.channel_picker else {
            return false;
        };
        let Some(&ei) = p.filtered.get(p.selected) else {
            return false;
        };
        let Some(entry) = p.entries.get(ei) else {
            return false;
        };
        self.selected_server = entry.server.clone();
        self.selected_channel_id = Some(entry.channel_id.clone());
        self.channel_picker = None;
        self.message_scroll_from_bottom = 0;
        self.selected_message_index = None;
        self.normalize_selection();
        true
    }

    pub fn dismiss_channel_picker(&mut self) {
        self.channel_picker = None;
    }

    pub fn confirm_reaction_emoji(&mut self) -> Option<(String, String, String)> {
        let (ch_id, msg_id) = self.reaction_target.clone()?;
        let auto = self.emoji_autocomplete.as_ref()?;
        let emoji = auto.matches.get(auto.selected_index)?;
        let api = encode_reaction_for_api(&emoji.insert);
        self.reaction_target = None;
        self.emoji_autocomplete = None;
        self.input.clear();
        Some((ch_id, msg_id, api))
    }

    // ma

    pub fn start_mention_autocomplete(&mut self) {
        let pool = self.build_mention_pool();
        if pool.is_empty() {
            return;
        }
        self.mention_autocomplete = Some(MentionAutocomplete {
            pool,
            matches: Vec::new(),
            selected_index: 0,
        });
        self.update_mention_filter();
    }

    /// repopulate @ UI if it was waiting
    pub fn refresh_mention_autocomplete_after_members_load(&mut self, loaded_guild_id: &str) {
        if self.mention_autocomplete.is_none() {
            return;
        }
        if self.active_guild_id().as_deref() != Some(loaded_guild_id) {
            return;
        }
        let pool = self.build_mention_pool();
        if pool.is_empty() {
            self.mention_autocomplete = None;
            return;
        }
        if let Some(auto) = &mut self.mention_autocomplete {
            auto.pool = pool;
            auto.selected_index = 0;
        }
        self.update_mention_filter();
        if self.mention_autocomplete.is_some() {
            self.clear_status();
        }
    }

    const MENTION_FILTER_CAP: usize = 400;
    const MENTION_INITIAL_CAP: usize = 80;

    fn build_mention_pool(&self) -> Vec<MentionPick> {
        let mut users: Vec<MentionPick> = Vec::new();
        let mut roles: Vec<MentionPick> = Vec::new();
        let mut seen_users: HashSet<String> = HashSet::new();

        match &self.selected_server {
            ServerSelection::Guild(guild_id) => {
                if let Some(members) = self.guild_members.get(guild_id) {
                    for m in members {
                        if m.user.id.is_empty() || !seen_users.insert(m.user.id.clone()) {
                            continue;
                        }
                        let cached_user = self.user_cache.get(&m.user.id);
                        let username = if !m.user.username.is_empty() {
                            m.user.username.clone()
                        } else {
                            cached_user.map(|u| u.username.clone()).unwrap_or_default()
                        };
                        let base_display = if !m.user.username.is_empty() {
                            account_display_name(cached_user.unwrap_or(&m.user))
                        } else {
                            cached_user.map(account_display_name).unwrap_or_default()
                        };
                        let nick_display = m
                            .nick
                            .clone()
                            .filter(|n| !n.trim().is_empty())
                            .unwrap_or(base_display);
                        if username.is_empty() && nick_display.is_empty() {
                            continue;
                        }
                        users.push(MentionPick::User {
                            user_id: m.user.id.clone(),
                            display: nick_display,
                            username,
                        });
                    }
                }
                if let Some(rs) = self.guild_roles.get(guild_id) {
                    let mut seen_roles: HashSet<String> = HashSet::new();
                    for r in rs {
                        if r.id.is_empty() || !seen_roles.insert(r.id.clone()) {
                            continue;
                        }
                        let name = if r.name.trim().is_empty() {
                            continue;
                        } else {
                            r.name.clone()
                        };
                        roles.push(MentionPick::Role {
                            role_id: r.id.clone(),
                            name,
                            color: r.color,
                        });
                    }
                }
                roles.sort_by(|a, b| match (a, b) {
                    (MentionPick::Role { name: na, .. }, MentionPick::Role { name: nb, .. }) => {
                        na.to_lowercase().cmp(&nb.to_lowercase())
                    }
                    _ => std::cmp::Ordering::Equal,
                });
            }
            ServerSelection::DirectMessages => {
                if let Some(ch) = self.active_channel() {
                    for r in &ch.recipients {
                        if r.id.is_empty() || !seen_users.insert(r.id.clone()) {
                            continue;
                        }
                        users.push(MentionPick::User {
                            user_id: r.id.clone(),
                            display: display_name(r),
                            username: r.username.clone(),
                        });
                    }
                }
            }
        }

        if let Some(channel_id) = self.selected_channel_id.as_deref() {
            let guild_id = self.guild_id_for_channel(channel_id);
            if let Some(msgs) = self.messages.get(channel_id) {
                for msg in msgs {
                    if msg.author.id.is_empty() || !seen_users.insert(msg.author.id.clone()) {
                        continue;
                    }
                    let nick = guild_id.as_ref().and_then(|gid| {
                        self.guild_members.get(gid).and_then(|mems| {
                            mems.iter()
                                .find(|m| m.user.id == msg.author.id)
                                .and_then(|m| m.nick.clone())
                        })
                    });
                    let u = self.user_cache.get(&msg.author.id).unwrap_or(&msg.author);
                    let base_display = account_display_name(u);
                    let display = nick
                        .filter(|n| !n.trim().is_empty())
                        .unwrap_or(base_display.clone());
                    let username = if !msg.author.username.is_empty() {
                        msg.author.username.clone()
                    } else {
                        self.user_cache
                            .get(&msg.author.id)
                            .map(|u| u.username.clone())
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| base_display.clone())
                    };
                    if username.is_empty() && display.is_empty() {
                        continue;
                    }
                    users.push(MentionPick::User {
                        user_id: msg.author.id.clone(),
                        display,
                        username,
                    });
                }
            }
        }

        users.sort_by(|a, b| match (a, b) {
            (MentionPick::User { display: da, .. }, MentionPick::User { display: db, .. }) => {
                da.to_lowercase().cmp(&db.to_lowercase())
            }
            _ => std::cmp::Ordering::Equal,
        });

        let mut pool = users;
        pool.extend(roles);
        pool
    }

    pub fn update_mention_filter(&mut self) {
        let Some(auto) = &mut self.mention_autocomplete else {
            return;
        };

        let query = self.input.rsplit('@').next().unwrap_or("").to_lowercase();

        auto.matches = if query.is_empty() {
            (0..auto.pool.len().min(Self::MENTION_INITIAL_CAP)).collect()
        } else {
            auto.pool
                .iter()
                .enumerate()
                .filter(|(_, p)| p.matches_filter(&query))
                .map(|(i, _)| i)
                .take(Self::MENTION_FILTER_CAP)
                .collect()
        };

        if auto.selected_index >= auto.matches.len() {
            auto.selected_index = auto.matches.len().saturating_sub(1);
        }
        if auto.matches.is_empty() {
            self.mention_autocomplete = None;
        }
    }

    pub fn dismiss_mention_autocomplete(&mut self) {
        self.mention_autocomplete = None;
    }

    pub fn autocomplete_mention_next(&mut self) {
        if let Some(auto) = &mut self.mention_autocomplete
            && !auto.matches.is_empty()
        {
            auto.selected_index = (auto.selected_index + 1) % auto.matches.len();
        }
    }

    pub fn autocomplete_mention_prev(&mut self) {
        if let Some(auto) = &mut self.mention_autocomplete
            && !auto.matches.is_empty()
        {
            auto.selected_index =
                auto.selected_index.saturating_add(auto.matches.len() - 1) % auto.matches.len();
        }
    }

    pub fn insert_selected_mention(&mut self) -> bool {
        if let Some(auto) = &self.mention_autocomplete
            && let Some(&pool_idx) = auto.matches.get(auto.selected_index)
            && let Some(pick) = auto.pool.get(pool_idx)
            && let Some(at_pos) = self.input.rfind('@')
        {
            self.input.truncate(at_pos);
            match pick {
                MentionPick::User { user_id, .. } => {
                    self.input.push_str(&format!("<@{user_id}> "));
                }
                MentionPick::Role { role_id, .. } => {
                    self.input.push_str(&format!("<@&{role_id}> "));
                }
            }
            self.mention_autocomplete = None;
            return true;
        }
        false
    }

    pub fn self_nick_or_username_in_guild(&self, guild_id: &str) -> String {
        self.shown_name_for_user(Some(guild_id), &me_as_partial(&self.me))
    }

    pub fn shown_name_for_user(
        &self,
        guild_id: Option<&str>,
        user: &UserPartialResponse,
    ) -> String {
        let u = self.user_cache.get(&user.id).unwrap_or(user);
        if let Some(gid) = guild_id {
            if let Some(members) = self.guild_members.get(gid) {
                if let Some(m) = members.iter().find(|m| m.user.id == user.id) {
                    let base = self.user_cache.get(&m.user.id).unwrap_or(&m.user);
                    return m
                        .nick
                        .as_ref()
                        .filter(|n| !n.trim().is_empty())
                        .cloned()
                        .unwrap_or_else(|| account_display_name(base));
                }
            }
        }
        account_display_name(u)
    }

    pub fn member_name_color(&self, guild_id: Option<&str>, user_id: &str, is_self: bool) -> Color {
        use crate::api::types::snowflake_sort_key;

        let guild_default = || crate::ui::theme::text();

        if let Some(gid) = guild_id {
            if let Some(members) = self.guild_members.get(gid) {
                if let Some(member) = members.iter().find(|m| m.user.id == user_id) {
                    if let Some(roles) = self.guild_roles.get(gid) {
                        let role_pos = |rid: &str| {
                            let rid = rid.trim();
                            roles
                                .iter()
                                .find(|r| r.id.trim() == rid)
                                .map(|r| r.position)
                                .unwrap_or(i32::MIN)
                        };
                        let mut role_ids: Vec<&str> =
                            member.roles.iter().map(|s| s.as_str()).collect();
                        role_ids.sort_by(|a, b| {
                            role_pos(b).cmp(&role_pos(a)).then_with(|| {
                                snowflake_sort_key(a.trim()).cmp(&snowflake_sort_key(b.trim()))
                            })
                        });
                        for rid in role_ids {
                            let rid = rid.trim();
                            if let Some(r) = roles.iter().find(|rr| rr.id.trim() == rid) {
                                if r.color != 0 {
                                    return crate::ui::theme::rgb_pack_to_color(r.color);
                                }
                            }
                        }
                        let gid_trim = gid.trim();
                        if let Some(everyone) = roles.iter().find(|r| r.id.trim() == gid_trim) {
                            if everyone.color != 0 {
                                return crate::ui::theme::rgb_pack_to_color(everyone.color);
                            }
                        }
                    }
                    return guild_default();
                }
            }
            return guild_default();
        }

        if is_self {
            crate::ui::theme::self_username_color()
        } else {
            crate::ui::theme::username_color(user_id)
        }
    }

    pub fn sync_command_autocomplete(&mut self) {
        if self.focus != Focus::Input || !self.can_send_in_active_channel() {
            self.command_autocomplete = None;
            return;
        }
        let Some(q) = crate::slash_commands::command_name_query(&self.input) else {
            self.command_autocomplete = None;
            return;
        };
        let guild_ch = self
            .active_channel()
            .and_then(|c| c.guild_id.clone())
            .is_some();
        let ch_perms = self.active_channel_permissions();
        let matches = crate::slash_commands::filter_command_indices(q, guild_ch, ch_perms);
        if matches.is_empty() {
            self.command_autocomplete = None;
            return;
        }
        let selected_index = self
            .command_autocomplete
            .as_ref()
            .map(|a| a.selected_index.min(matches.len().saturating_sub(1)))
            .unwrap_or(0);
        self.command_autocomplete = Some(CommandAutocomplete {
            matches,
            selected_index,
        });
    }

    pub fn dismiss_command_autocomplete(&mut self) {
        self.command_autocomplete = None;
    }

    pub fn autocomplete_command_next(&mut self) {
        if let Some(auto) = &mut self.command_autocomplete
            && !auto.matches.is_empty()
        {
            auto.selected_index = (auto.selected_index + 1) % auto.matches.len();
        }
    }

    pub fn autocomplete_command_prev(&mut self) {
        if let Some(auto) = &mut self.command_autocomplete
            && !auto.matches.is_empty()
        {
            auto.selected_index =
                auto.selected_index.saturating_add(auto.matches.len() - 1) % auto.matches.len();
        }
    }

    pub fn insert_selected_slash_command(&mut self) -> bool {
        let Some(auto) = &self.command_autocomplete else {
            return false;
        };
        let Some(&cmd_i) = auto.matches.get(auto.selected_index) else {
            return false;
        };
        let cmd = &crate::slash_commands::SLASH_COMMANDS[cmd_i];
        let Some(slash_pos) = self.input.find('/') else {
            return false;
        };
        let after = &self.input[slash_pos + 1..];
        let token_len = after
            .find(|c: char| c.is_whitespace())
            .unwrap_or(after.len());
        let end = slash_pos + 1 + token_len;
        let trailing_space = cmd.simple_append.is_none();
        let mut new_in = String::new();
        new_in.push_str(&self.input[..slash_pos]);
        new_in.push_str(cmd.name);
        if trailing_space {
            new_in.push(' ');
        }
        new_in.push_str(&self.input[end..]);
        self.input = new_in;
        self.command_autocomplete = None;
        self.sync_command_autocomplete();
        true
    }

    fn merge_single_message_member(&mut self, message: &MessageResponse) {
        let Some(mem) = message.member.as_ref() else {
            return;
        };
        let Some(gid) = self.guild_id_for_channel(&message.channel_id) else {
            return;
        };
        self.merge_guild_member(gid.as_str(), mem.clone());
    }

    fn merge_message_embedded_members(&mut self, message: &MessageResponse) {
        self.merge_single_message_member(message);
        if let Some(r) = message.referenced_message.as_deref() {
            self.merge_message_embedded_members(r);
        }
    }

    pub fn merge_guild_member(&mut self, guild_id: &str, mut member: GuildMemberResponse) {
        self.roster_version = self.roster_version.wrapping_add(1);
        merge_user_cache(&mut self.user_cache, [member.user.clone()]);
        let members = self.guild_members.entry(guild_id.to_string()).or_default();
        if let Some(existing) = members.iter().find(|m| m.user.id == member.user.id) {
            if member.roles.is_empty() && !existing.roles.is_empty() {
                member.roles = existing.roles.clone();
            }
            if member.nick.is_none() && existing.nick.is_some() {
                member.nick = existing.nick.clone();
            }
        }
        if let Some(existing) = members.iter_mut().find(|m| m.user.id == member.user.id) {
            *existing = member;
        } else {
            members.push(member);
        }
    }

    pub fn allocate_local_message_snowflake(&self, channel_id: &str) -> String {
        let msgs = self
            .messages
            .get(channel_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        let max_k = msgs
            .iter()
            .map(|m| snowflake_sort_key(&m.id))
            .max()
            .unwrap_or(0);
        const DISCORD_EPOCH_MS: u128 = 1420070400000;
        let ts = chrono::Utc::now().timestamp_millis() as u128;
        let delta = ts.saturating_sub(DISCORD_EPOCH_MS);
        let mut candidate = delta.saturating_mul(1u128 << 22);
        if candidate <= max_k {
            candidate = max_k.saturating_add(1);
        }
        candidate.to_string()
    }

    // permission(orn) helpers

    pub fn can_react_in_active_channel(&self) -> bool {
        self.active_channel_permissions() & crate::permissions::ADD_REACTIONS != 0
    }
}

fn picker_channel_line(ch: &ChannelResponse) -> String {
    match ch.channel_type() {
        CHANNEL_GUILD_TEXT | CHANNEL_GUILD_LINK => {
            if ch.name.is_empty() {
                ch.id.chars().take(6).collect()
            } else {
                format!("#{}", ch.name)
            }
        }
        CHANNEL_DM_PERSONAL_NOTES => "Personal notes".to_string(),
        CHANNEL_DM => ch
            .recipients
            .first()
            .map(display_name)
            .unwrap_or_else(|| "DM".to_string()),
        CHANNEL_GROUP_DM => {
            if !ch.name.trim().is_empty() {
                ch.name.clone()
            } else if !ch.recipients.is_empty() {
                ch.recipients
                    .iter()
                    .map(display_name)
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                "Group DM".to_string()
            }
        }
        _ => {
            if ch.name.is_empty() {
                ch.id.chars().take(6).collect()
            } else {
                ch.name.clone()
            }
        }
    }
}

fn encode_reaction_for_api(insert: &str) -> String {
    let t = insert.trim();
    if t.starts_with('<') && t.ends_with('>') && t.contains(':') {
        let inner = &t[1..t.len() - 1];
        let parts: Vec<&str> = inner.split(':').collect();
        if parts.len() >= 3 {
            let id = parts[parts.len() - 1];
            let name = parts[parts.len() - 2];
            if !name.is_empty() && !id.is_empty() {
                return format!("{name}:{id}");
            }
        }
    }
    t.to_string()
}

pub fn me_as_partial(me: &UserPrivateResponse) -> UserPartialResponse {
    UserPartialResponse {
        id: me.id.clone(),
        username: me.username.clone(),
        discriminator: me.discriminator.clone(),
        global_name: me.global_name.clone(),
        avatar: me.avatar.clone(),
        avatar_color: me.avatar_color,
        bot: me.bot,
        system: me.system,
    }
}

pub fn account_display_name(user: &UserPartialResponse) -> String {
    user.global_name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| username_handle(user))
}

fn username_handle(user: &UserPartialResponse) -> String {
    if user.discriminator.is_empty() {
        user.username.clone()
    } else {
        format!("{}#{}", user.username, user.discriminator)
    }
}

pub fn display_name(user: &UserPartialResponse) -> String {
    account_display_name(user)
}

fn fluxer_typing_phrase(names: &[String]) -> String {
    const SEVERAL: &str = "Several people are typing...";
    const HANDFUL: &str = "A handful of keyboard warriors are assembling...";
    const SYMPHONY: &str = "A symphony of clacking keys is underway...";
    const FIESTA: &str = "It's a full-blown typing fiesta in here";
    const APOCALYPSE: &str = "Whoa, it's a typing apocalypse";

    match names.len() {
        1 => format!("{} is typing...", names[0]),
        2 => format!("{} and {} are typing...", names[0], names[1]),
        3 => format!("{}, {} and {} are typing...", names[0], names[1], names[2]),
        4 => SEVERAL.to_string(),
        n if (5..=9).contains(&n) => HANDFUL.to_string(),
        n if (10..=14).contains(&n) => SYMPHONY.to_string(),
        n if (15..=19).contains(&n) => FIESTA.to_string(),
        _ => APOCALYPSE.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app(private_channels: Vec<ChannelResponse>) -> App {
        let mut me = UserPrivateResponse::default();
        me.id = "me".to_string();

        App::new(
            WellKnownFluxerResponse::default(),
            me,
            None,
            Vec::new(),
            private_channels,
            ServerSelection::DirectMessages,
            None,
            UiSettings::default(),
        )
    }

    fn guild_app(channels: Vec<ChannelResponse>) -> App {
        let mut app = test_app(Vec::new());
        app.guilds.push(GuildResponse {
            id: "guild-1".to_string(),
            name: "Guild".to_string(),
            ..GuildResponse::default()
        });
        app.guild_channels.insert("guild-1".to_string(), channels);
        app.selected_server = ServerSelection::Guild("guild-1".to_string());
        app
    }

    #[test]
    fn channel_last_message_id_prefers_newer_channel_metadata() {
        let channel = ChannelResponse {
            id: "dm-1".to_string(),
            kind: CHANNEL_DM,
            last_message_id: Some("300".to_string()),
            ..ChannelResponse::default()
        };
        let mut app = test_app(vec![channel]);
        app.messages.insert(
            "dm-1".to_string(),
            vec![
                MessageResponse {
                    id: "100".to_string(),
                    channel_id: "dm-1".to_string(),
                    ..MessageResponse::default()
                },
                MessageResponse {
                    id: "250".to_string(),
                    channel_id: "dm-1".to_string(),
                    ..MessageResponse::default()
                },
            ],
        );

        assert_eq!(app.channel_last_message_id("dm-1").as_deref(), Some("300"));
    }

    #[test]
    fn ack_channel_uses_channel_metadata_without_cached_messages() {
        let channel = ChannelResponse {
            id: "dm-1".to_string(),
            kind: CHANNEL_DM,
            last_message_id: Some("400".to_string()),
            ..ChannelResponse::default()
        };
        let mut app = test_app(vec![channel]);

        app.ack_channel("dm-1");

        assert_eq!(
            app.read_states
                .get("dm-1")
                .and_then(|state| state.last_message_id.as_deref()),
            Some("400")
        );
        assert_eq!(app.channel_mention_count("dm-1"), 0);
    }

    #[test]
    fn auto_load_history_only_near_top_when_enabled() {
        let mut app = test_app(Vec::new());
        app.message_scroll_max = 24;

        app.message_scroll_from_bottom = 20;
        assert!(!app.should_auto_load_history_on_scroll_up());

        app.message_scroll_from_bottom = 21;
        assert!(app.should_auto_load_history_on_scroll_up());
    }

    #[test]
    fn auto_load_history_uses_scroll_threshold() {
        let mut app = test_app(Vec::new());
        app.message_scroll_max = 24;
        app.message_scroll_from_bottom = 18;

        assert!(!app.should_auto_load_history_on_scroll_up());
    }

    #[test]
    fn server_aggregates_unread_channels_and_mentions() {
        let mut app = test_app(Vec::new());
        app.guilds.push(GuildResponse {
            id: "guild-1".to_string(),
            name: "Guild".to_string(),
            ..GuildResponse::default()
        });
        app.guild_channels.insert(
            "guild-1".to_string(),
            vec![
                ChannelResponse {
                    id: "chan-1".to_string(),
                    guild_id: Some("guild-1".to_string()),
                    name: "alpha".to_string(),
                    kind: CHANNEL_GUILD_TEXT,
                    last_message_id: Some("200".to_string()),
                    ..ChannelResponse::default()
                },
                ChannelResponse {
                    id: "chan-2".to_string(),
                    guild_id: Some("guild-1".to_string()),
                    name: "beta".to_string(),
                    kind: CHANNEL_GUILD_TEXT,
                    last_message_id: Some("300".to_string()),
                    ..ChannelResponse::default()
                },
            ],
        );
        app.read_states.insert(
            "chan-1".to_string(),
            ReadState {
                last_message_id: Some("150".to_string()),
                mention_count: 0,
            },
        );
        app.read_states.insert(
            "chan-2".to_string(),
            ReadState {
                last_message_id: Some("300".to_string()),
                mention_count: 2,
            },
        );

        let server = ServerSelection::Guild("guild-1".to_string());
        assert_eq!(app.server_unread_channel_count(&server), 1);
        assert_eq!(app.server_mention_count(&server), 2);
    }

    #[test]
    fn muted_server_hides_unread_only_activity() {
        let mut app = guild_app(vec![
            ChannelResponse {
                id: "chan-1".to_string(),
                guild_id: Some("guild-1".to_string()),
                name: "alpha".to_string(),
                kind: CHANNEL_GUILD_TEXT,
                position: 1,
                last_message_id: Some("200".to_string()),
                ..ChannelResponse::default()
            },
            ChannelResponse {
                id: "chan-2".to_string(),
                guild_id: Some("guild-1".to_string()),
                name: "beta".to_string(),
                kind: CHANNEL_GUILD_TEXT,
                position: 2,
                last_message_id: Some("300".to_string()),
                ..ChannelResponse::default()
            },
        ]);
        app.read_states.insert(
            "chan-1".to_string(),
            ReadState {
                last_message_id: Some("150".to_string()),
                mention_count: 0,
            },
        );
        app.read_states.insert(
            "chan-2".to_string(),
            ReadState {
                last_message_id: Some("300".to_string()),
                mention_count: 2,
            },
        );
        app.upsert_user_guild_settings(UserGuildSettingsResponse {
            guild_id: Some("guild-1".to_string()),
            muted: true,
            ..UserGuildSettingsResponse::default()
        });

        let server = ServerSelection::Guild("guild-1".to_string());
        assert_eq!(app.server_unread_channel_count(&server), 0);
        assert_eq!(app.server_mention_count(&server), 2);
    }

    #[test]
    fn hide_muted_channels_filters_sidebar_entries() {
        let mut app = guild_app(vec![
            ChannelResponse {
                id: "chan-1".to_string(),
                guild_id: Some("guild-1".to_string()),
                name: "alpha".to_string(),
                kind: CHANNEL_GUILD_TEXT,
                position: 1,
                ..ChannelResponse::default()
            },
            ChannelResponse {
                id: "chan-2".to_string(),
                guild_id: Some("guild-1".to_string()),
                name: "beta".to_string(),
                kind: CHANNEL_GUILD_TEXT,
                position: 2,
                ..ChannelResponse::default()
            },
        ]);
        app.selected_channel_id = Some("chan-2".to_string());
        app.upsert_user_guild_settings(UserGuildSettingsResponse {
            guild_id: Some("guild-1".to_string()),
            hide_muted_channels: true,
            channel_overrides: HashMap::from([(
                "chan-1".to_string(),
                UserGuildChannelOverride {
                    muted: true,
                    ..UserGuildChannelOverride::default()
                },
            )]),
            ..UserGuildSettingsResponse::default()
        });

        let visible = app.channels_for_server(&ServerSelection::Guild("guild-1".to_string()));
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, "chan-2");
    }

    #[test]
    fn suppress_everyone_blocks_local_mention_increment() {
        let mut app = guild_app(vec![ChannelResponse {
            id: "chan-1".to_string(),
            guild_id: Some("guild-1".to_string()),
            name: "alpha".to_string(),
            kind: CHANNEL_GUILD_TEXT,
            position: 1,
            last_message_id: Some("200".to_string()),
            ..ChannelResponse::default()
        }]);
        app.upsert_user_guild_settings(UserGuildSettingsResponse {
            guild_id: Some("guild-1".to_string()),
            suppress_everyone: true,
            ..UserGuildSettingsResponse::default()
        });
        app.read_states.insert(
            "chan-1".to_string(),
            ReadState {
                last_message_id: Some("150".to_string()),
                mention_count: 0,
            },
        );

        app.on_gateway_message_create(&MessageResponse {
            id: "250".to_string(),
            channel_id: "chan-1".to_string(),
            mention_everyone: true,
            author: UserPartialResponse {
                id: "other".to_string(),
                ..UserPartialResponse::default()
            },
            ..MessageResponse::default()
        });

        assert_eq!(app.channel_mention_count("chan-1"), 0);
    }
}

/// A `<:name:id>` / `<a:name:id>` token at the start of `s`.
#[derive(Debug, PartialEq, Eq)]
pub struct CustomEmojiToken<'a> {
    pub name: &'a str,
    pub id: &'a str,
    pub animated: bool,
    /// Byte length of the whole token including the angle brackets.
    pub len: usize,
}

pub fn parse_custom_emoji_token(s: &str) -> Option<CustomEmojiToken<'_>> {
    let body = s.strip_prefix('<')?;
    let (animated, body) = match body.strip_prefix("a:") {
        Some(rest) => (true, rest),
        None => (false, body.strip_prefix(':')?),
    };
    let close = body.find('>')?;
    let inner = &body[..close];
    let (name, id) = inner.rsplit_once(':')?;
    if name.is_empty()
        || id.is_empty()
        || !id.bytes().all(|b| b.is_ascii_digit())
        || name.contains(|c: char| c.is_whitespace() || c == '<' || c == ':')
    {
        return None;
    }
    Some(CustomEmojiToken {
        name,
        id,
        animated,
        len: 1 + if animated { 2 } else { 1 } + close + 1,
    })
}

/// Placeholder cells carry their slot number in the underline colour, which
/// is invisible without an underline and survives Paragraph wrapping.
pub fn custom_emoji_marker_style(slot: usize) -> Style {
    let k = slot.min(u16::MAX as usize) as u16;
    Style::default().underline_color(Color::Rgb(0xEE, (k >> 8) as u8, k as u8))
}

pub fn custom_emoji_marker_slot(style: Style) -> Option<usize> {
    match style.underline_color {
        Some(Color::Rgb(0xEE, hi, lo)) => Some(((hi as usize) << 8) | lo as usize),
        _ => None,
    }
}

/// Media block cells carry their row within the block in the red channel
/// (0xD0 + row, so 16 rows at most) and the slot in green and blue.
pub fn media_marker_style(slot: usize, row: u16) -> Style {
    let k = slot.min(u16::MAX as usize) as u16;
    Style::default().underline_color(Color::Rgb(
        0xD0 | (row.min(15) as u8),
        (k >> 8) as u8,
        k as u8,
    ))
}

pub fn media_marker(style: Style) -> Option<(usize, u16)> {
    match style.underline_color {
        Some(Color::Rgb(r, hi, lo)) if r & 0xF0 == 0xD0 => {
            Some((((hi as usize) << 8) | lo as usize, (r & 0x0F) as u16))
        }
        _ => None,
    }
}

/// The cell a terminal picture is printed at: the backend prints the
/// picture there instead of the cell. The frames' serial and the frame
/// index sit in the underline colour (red 0x80..0xBF), so the cell changes
/// exactly when the picture does: a still picture is sent once, an
/// animation once per frame.
pub fn picture_sentinel_style(style: Style, serial: u16, frame: usize) -> Style {
    style.underline_color(Color::Rgb(
        0x80 | (frame as u8 & 0x3F),
        (serial >> 8) as u8,
        serial as u8,
    ))
}

pub fn is_picture_sentinel(underline_color: Color) -> bool {
    matches!(underline_color, Color::Rgb(r, _, _) if r & 0xC0 == 0x80)
}

#[cfg(test)]
mod custom_emoji_tests {
    use super::*;

    #[test]
    fn marker_style_round_trips_slot_numbers() {
        for k in [0usize, 1, 7, 255, 256, 4095, 65535] {
            assert_eq!(
                custom_emoji_marker_slot(custom_emoji_marker_style(k)),
                Some(k)
            );
        }
        assert_eq!(custom_emoji_marker_slot(Style::default()), None);
        assert_eq!(
            custom_emoji_marker_slot(Style::default().underline_color(Color::Rgb(1, 2, 3))),
            None
        );
    }

    #[test]
    fn picture_sentinels_are_not_markers() {
        let s = picture_sentinel_style(Style::default(), 0x1234, 47);
        assert!(is_picture_sentinel(s.underline_color.unwrap()));
        assert_eq!(s.underline_color, Some(Color::Rgb(0x80 | 47, 0x12, 0x34)));
        assert_eq!(media_marker(s), None);
        assert_eq!(custom_emoji_marker_slot(s), None);
        assert!(!is_picture_sentinel(
            media_marker_style(3, 2).underline_color.unwrap()
        ));
        assert!(!is_picture_sentinel(
            custom_emoji_marker_style(3).underline_color.unwrap()
        ));
        assert!(!is_picture_sentinel(Color::Reset));
    }

    #[test]
    fn media_marker_round_trips_slot_and_row() {
        for (k, r) in [(0usize, 0u16), (1, 1), (300, 11), (65535, 15)] {
            assert_eq!(media_marker(media_marker_style(k, r)), Some((k, r)));
        }
        assert_eq!(media_marker(Style::default()), None);
        assert_eq!(media_marker(custom_emoji_marker_style(3)), None);
        assert_eq!(custom_emoji_marker_slot(media_marker_style(3, 0)), None);
    }

    #[test]
    fn custom_emoji_tokens_parse() {
        let t = parse_custom_emoji_token("<:blob:123> tail").unwrap();
        assert_eq!(
            (t.name, t.id, t.animated, t.len),
            ("blob", "123", false, 11)
        );
        let t = parse_custom_emoji_token("<a:party:9>").unwrap();
        assert_eq!((t.name, t.id, t.animated, t.len), ("party", "9", true, 11));
        assert!(parse_custom_emoji_token("<:blob:abc>").is_none());
        assert!(parse_custom_emoji_token("<#123>").is_none());
        assert!(parse_custom_emoji_token("<:no close").is_none());
        assert!(parse_custom_emoji_token("<::1>").is_none());
    }

    #[test]
    fn animation_timeline_loops() {
        let f = PictureFrames::new(
            Vec::new(),
            vec![
                Duration::from_millis(100),
                Duration::from_millis(50),
                Duration::from_millis(150),
            ],
        );
        let t0 = Instant::now();
        let mut draw = 0u64;
        for (ms, want) in [(0u64, 0usize), (100, 1), (150, 2), (300, 0), (1000, 1)] {
            draw += 1;
            assert_eq!(
                f.index_at(t0 + Duration::from_millis(ms), draw),
                want,
                "at {ms} ms"
            );
        }
    }

    #[test]
    fn a_long_frame_is_held_and_never_shown_twice() {
        // three quick frames and a half-second pause: the pause is held
        // for as many draws as it takes, without stepping ahead and back
        let f = PictureFrames::new(
            Vec::new(),
            vec![
                Duration::from_millis(100),
                Duration::from_millis(100),
                Duration::from_millis(100),
                Duration::from_millis(500),
            ],
        );
        let t0 = Instant::now();
        let seen: Vec<usize> = (0..10u64)
            .map(|tick| f.index_at(t0 + Duration::from_millis(100 * tick), tick + 1))
            .collect();
        assert_eq!(seen, vec![0, 1, 2, 3, 3, 3, 3, 3, 0, 1]);
    }

    #[test]
    fn fast_loops_still_advance() {
        // two 50 ms frames: a 100 ms loop that a 100 ms tick would always
        // sample at the same frame
        let f = PictureFrames::new(
            Vec::new(),
            vec![Duration::from_millis(50), Duration::from_millis(50)],
        );
        let t0 = Instant::now();
        let seen: Vec<usize> = (0..6u64)
            .map(|tick| f.index_at(t0 + Duration::from_millis(100 * tick), tick + 1))
            .collect();
        assert_eq!(seen, vec![0, 1, 0, 1, 0, 1]);
        // redraws inside the same frame's time (typing, cursor blink) do not advance
        let now = t0 + Duration::from_millis(520);
        let a = f.index_at(now, 10);
        let b = f.index_at(now + Duration::from_millis(5), 11);
        assert_eq!(a, b);
    }

    #[test]
    fn irregular_draws_keep_the_cadence() {
        // 40 ms frames drawn every 50 ms: five frames go by in every four
        // draws, never the same frame twice in a row, never a frame skipped
        // twice in a row
        let f = PictureFrames::new(Vec::new(), vec![Duration::from_millis(40); 10]);
        let t0 = Instant::now();
        let seen: Vec<usize> = (0..8u64)
            .map(|tick| f.index_at(t0 + Duration::from_millis(50 * tick), tick + 1))
            .collect();
        assert_eq!(seen, vec![0, 1, 2, 3, 5, 6, 7, 8]);
    }

    #[test]
    fn several_instances_in_one_draw_share_a_frame() {
        let f = PictureFrames::new(
            Vec::new(),
            vec![Duration::from_millis(50), Duration::from_millis(50)],
        );
        let t0 = Instant::now();
        let mut per_draw = Vec::new();
        for draw in 1..=6u64 {
            let now = t0 + Duration::from_millis(100 * draw);
            // the same emoji drawn three times in this draw
            let a = f.index_at(now, draw);
            let b = f.index_at(now, draw);
            let c = f.index_at(now, draw);
            assert_eq!(b, a);
            assert_eq!(c, a);
            per_draw.push(a);
        }
        // and it still moves from draw to draw
        assert!(per_draw.windows(2).all(|w| w[0] != w[1]), "{per_draw:?}");
    }

    #[test]
    fn placeholder_is_two_cells_and_not_whitespace() {
        use unicode_width::UnicodeWidthStr;
        assert_eq!(
            CUSTOM_EMOJI_PLACEHOLDER.width(),
            CUSTOM_EMOJI_CELLS as usize
        );
        assert!(!CUSTOM_EMOJI_PLACEHOLDER.chars().any(char::is_whitespace));
    }
}
