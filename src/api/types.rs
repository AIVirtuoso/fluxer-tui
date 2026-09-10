use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::HashMap;

fn deserialize_role_color<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    use std::fmt;

    struct RoleColorVisitor;
    impl<'de> Visitor<'de> for RoleColorVisitor {
        type Value = u32;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("integer or string role color")
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<u32, E> {
            Ok(v as u32)
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<u32, E> {
            Ok(v as u32)
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<u32, E> {
            let n = v.trim().parse::<i64>().map_err(de::Error::custom)?;
            Ok(n as u32)
        }

        fn visit_f64<E: de::Error>(self, v: f64) -> Result<u32, E> {
            Ok(v as u32)
        }

        fn visit_none<E>(self) -> Result<u32, E> {
            Ok(0)
        }

        fn visit_unit<E>(self) -> Result<u32, E> {
            Ok(0)
        }
    }

    deserializer.deserialize_any(RoleColorVisitor)
}

fn deserialize_snowflake_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    use std::fmt;

    struct SnowflakeStr;
    impl<'de> Visitor<'de> for SnowflakeStr {
        type Value = String;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("snowflake string or integer")
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<String, E> {
            Ok(v.to_string())
        }

        fn visit_string<E>(self, v: String) -> Result<String, E> {
            Ok(v)
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<String, E> {
            Ok(v.to_string())
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<String, E> {
            Ok(v.to_string())
        }
    }

    deserializer.deserialize_any(SnowflakeStr)
}

fn deserialize_vec_member_roles<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let v: Value = Deserialize::deserialize(deserializer)?;
    let Value::Array(arr) = v else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for el in arr {
        match el {
            Value::String(s) => out.push(s),
            Value::Number(n) => {
                if let Some(u) = n.as_u64() {
                    out.push(u.to_string());
                } else if let Some(i) = n.as_i64() {
                    out.push(i.to_string());
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

fn deserialize_i32_flex<'de, D>(deserializer: D) -> Result<i32, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    use std::fmt;

    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = i32;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("i32 or numeric string")
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<i32, E> {
            Ok(v as i32)
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<i32, E> {
            Ok(v as i32)
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<i32, E> {
            v.trim().parse::<i32>().map_err(de::Error::custom)
        }

        fn visit_f64<E: de::Error>(self, v: f64) -> Result<i32, E> {
            Ok(v as i32)
        }
    }

    deserializer.deserialize_any(V)
}

pub type Snowflake = String;

pub const CHANNEL_GUILD_TEXT: i32 = 0;
pub const CHANNEL_DM: i32 = 1;
pub const CHANNEL_GUILD_VOICE: i32 = 2;
pub const CHANNEL_GROUP_DM: i32 = 3;
pub const CHANNEL_GUILD_CATEGORY: i32 = 4;
pub const CHANNEL_GUILD_LINK: i32 = 998;
pub const CHANNEL_DM_PERSONAL_NOTES: i32 = 999;
pub const MESSAGE_NOTIFICATIONS_ALL_MESSAGES: i32 = 0;
pub const MESSAGE_NOTIFICATIONS_ONLY_MENTIONS: i32 = 1;
pub const MESSAGE_NOTIFICATIONS_NO_MESSAGES: i32 = 2;
pub const MESSAGE_NOTIFICATIONS_INHERIT: i32 = 3;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WellKnownFluxerResponse {
    #[serde(default)]
    pub api_code_version: u64,
    #[serde(default)]
    pub endpoints: WellKnownEndpoints,
    #[serde(default)]
    pub features: WellKnownFeatures,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WellKnownEndpoints {
    #[serde(default)]
    pub api: String,
    #[serde(default)]
    pub gateway: String,
    #[serde(default)]
    pub media: String,
    /// Static assets of the web app, such as the default avatars.
    #[serde(default)]
    pub static_cdn: String,
    #[serde(default)]
    pub webapp: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WellKnownFeatures {
    #[serde(default)]
    pub voice_enabled: bool,
    #[serde(default)]
    pub sms_mfa_enabled: bool,
    #[serde(default)]
    pub self_hosted: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HandoffInitiateResponse {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub expires_at: String,
    /// Secret issued alongside the code; the status endpoint only releases
    /// the token to a poller that presents it (server change of 2026-09-02).
    #[serde(default)]
    pub poll_secret: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HandoffStatusResponse {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GatewayBotResponse {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub shards: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserPrivateResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub discriminator: String,
    #[serde(default)]
    pub global_name: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    /// Colour of the default avatar (0xRRGGBB) when there is no picture.
    #[serde(default)]
    pub avatar_color: Option<u32>,
    #[serde(default)]
    pub bot: bool,
    #[serde(default)]
    pub system: bool,
    #[serde(default)]
    pub verified: bool,
    #[serde(default)]
    pub email: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct UserPartialResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub discriminator: String,
    #[serde(default)]
    pub global_name: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    /// Colour of the default avatar (0xRRGGBB) when there is no picture.
    #[serde(default)]
    pub avatar_color: Option<u32>,
    #[serde(default)]
    pub bot: bool,
    #[serde(default)]
    pub system: bool,
    /// Public account flags (staff, partner, bug hunter, ...).
    #[serde(default)]
    pub flags: u64,
}

/// Bits of `UserPartialResponse::flags` shown as badges.
pub mod user_flags {
    pub const STAFF: u64 = 1 << 0;
    pub const PARTNER: u64 = 1 << 2;
    pub const BUG_HUNTER: u64 = 1 << 3;
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuildResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub owner_id: String,
    #[serde(default)]
    pub permissions: Option<String>,
    #[serde(default)]
    pub default_message_notifications: i32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct GuildMemberResponse {
    #[serde(default)]
    pub user: UserPartialResponse,
    #[serde(default)]
    pub nick: Option<String>,
    /// Guild-specific avatar hash, shown instead of the user's own.
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default, deserialize_with = "deserialize_vec_member_roles")]
    pub roles: Vec<String>,
    #[serde(default)]
    pub mute: bool,
    #[serde(default)]
    pub deaf: bool,
    /// ISO 8601, when the member joined.
    #[serde(default)]
    pub joined_at: Option<String>,
}

/// The customisable part of a profile: the user's own, or their
/// guild-specific one.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileDataResponse {
    #[serde(default)]
    pub bio: Option<String>,
    #[serde(default)]
    pub pronouns: Option<String>,
    #[serde(default)]
    pub banner: Option<String>,
    #[serde(default)]
    pub accent_color: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MutualGuildResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub nick: Option<String>,
}

/// A verified external account shown on a profile.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConnectionResponse {
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub verified: bool,
}

/// `GET /users/{id}/profile`: what the web app's profile popup shows.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserProfileResponse {
    #[serde(default)]
    pub user: UserPartialResponse,
    #[serde(default)]
    pub user_profile: ProfileDataResponse,
    /// Only with `guild_id`, and only while they are a member.
    #[serde(default)]
    pub guild_member: Option<GuildMemberResponse>,
    #[serde(default)]
    pub guild_member_profile: Option<ProfileDataResponse>,
    /// 0 none, 1 subscription, 2 lifetime.
    #[serde(default)]
    pub premium_type: Option<u8>,
    #[serde(default)]
    pub premium_since: Option<String>,
    #[serde(default)]
    pub premium_lifetime_sequence: Option<i32>,
    #[serde(default)]
    pub mutual_friends: Option<Vec<UserPartialResponse>>,
    #[serde(default)]
    pub mutual_guilds: Option<Vec<MutualGuildResponse>>,
    #[serde(default)]
    pub connected_accounts: Option<Vec<ConnectionResponse>>,
    /// Minutes from UTC of the profile's time zone, when shared.
    #[serde(default)]
    pub timezone_offset: Option<i32>,
    /// The user restricted their profile: bio, pronouns, badges and
    /// connections were stripped.
    #[serde(default)]
    pub profile_limited: Option<bool>,
}

impl UserProfileResponse {
    /// The bio, pronouns and accent colour for the guild the profile was
    /// asked for, falling back field by field to the user's own.
    pub fn shown_profile(&self) -> ProfileDataResponse {
        let base = &self.user_profile;
        let Some(g) = self.guild_member_profile.as_ref() else {
            return base.clone();
        };
        let pick = |a: &Option<String>, b: &Option<String>| {
            a.as_ref()
                .filter(|s| !s.trim().is_empty())
                .or(b.as_ref())
                .cloned()
        };
        ProfileDataResponse {
            bio: pick(&g.bio, &base.bio),
            pronouns: pick(&g.pronouns, &base.pronouns),
            banner: pick(&g.banner, &base.banner),
            accent_color: g.accent_color.or(base.accent_color),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub guild_id: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub owner_id: Option<String>,
    #[serde(default)]
    pub kind: i32,
    #[serde(default, rename = "type")]
    pub raw_kind: i32,
    #[serde(default)]
    pub position: i32,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub bitrate: Option<i32>,
    #[serde(default)]
    pub user_limit: Option<i32>,
    #[serde(default)]
    pub rtc_region: Option<String>,
    #[serde(default)]
    pub last_message_id: Option<String>,
    #[serde(default)]
    pub recipients: Vec<UserPartialResponse>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub permission_overwrites: Vec<PermissionOverwrite>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PermissionOverwrite {
    #[serde(default)]
    pub id: String,
    #[serde(default, rename = "type")]
    pub kind: i32,
    #[serde(default)]
    pub allow: String,
    #[serde(default)]
    pub deny: String,
}

impl ChannelResponse {
    pub fn channel_type(&self) -> i32 {
        if self.raw_kind != 0 || self.kind == 0 {
            self.raw_kind
        } else {
            self.kind
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct MessageAttachmentResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub proxy_url: Option<String>,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    /// Pixel size of pictures and videos, known before anything is downloaded.
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    /// Length of an audio file in seconds.
    #[serde(default)]
    pub duration: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct EmbedMediaResponse {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub proxy_url: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    /// Bit 5 (32): the picture is animated.
    #[serde(default)]
    pub flags: Option<u32>,
}

impl EmbedMediaResponse {
    pub fn is_animated(&self) -> bool {
        self.flags.unwrap_or(0) & 32 != 0
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuildEmojiResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub animated: bool,
}

/// A sticker as its guild stores it: what the picker lists.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuildStickerResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Empty rather than null on the wire, but read forgivingly.
    #[serde(default)]
    pub description: Option<String>,
    /// Words the sticker is also found by; the picker searches them.
    #[serde(default, deserialize_with = "deserialize_lenient_vec")]
    pub tags: Vec<String>,
    #[serde(default)]
    pub animated: bool,
}

/// A sticker as a message carries it: the stored record trimmed down.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct MessageStickerResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub animated: bool,
    #[serde(default)]
    pub nsfw: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuildRoleResponse {
    #[serde(default, deserialize_with = "deserialize_snowflake_string")]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, deserialize_with = "deserialize_role_color", alias = "colour")]
    pub color: u32,
    #[serde(default, deserialize_with = "deserialize_i32_flex")]
    pub position: i32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct ReactionEmojiResponse {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub animated: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct MessageReactionResponse {
    #[serde(default)]
    pub emoji: ReactionEmojiResponse,
    #[serde(default)]
    pub count: u64,
    #[serde(default)]
    pub me: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct MessageReferenceResponse {
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub guild_id: Option<String>,
    #[serde(default, rename = "type")]
    pub reference_type: i32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageReferenceRequest {
    pub message_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub reference_type: Option<i32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReadStateResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub last_message_id: Option<String>,
    #[serde(default)]
    pub mention_count: u64,
}

/// A number, or a number spelled as a string; anything else is `None`.
fn lenient_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}

// The fields below are read the forgiving way. The API answers a settings
// update with `channel_overrides: null` when there are none, and the
// gateway spells some of the same fields differently (a list of overrides
// carrying `channel_id`, numbers as strings, `null` for unset). serde
// rejects the whole payload over one such field: the update looked like
// it had failed even though the server had saved it, and a READY with a
// saved entry in it left the client empty at the next start.

fn deserialize_lenient_bool<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(match &value {
        Value::Bool(b) => *b,
        Value::String(s) => matches!(s.trim(), "true" | "1"),
        other => lenient_i64(other).is_some_and(|n| n != 0),
    })
}

fn deserialize_lenient_i32<'de, D: Deserializer<'de>>(d: D) -> Result<i32, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(lenient_i64(&value).map(|n| n as i32).unwrap_or(0))
}

/// `null` means "not set", which is "follow the community's default".
fn deserialize_notification_level<'de, D: Deserializer<'de>>(d: D) -> Result<i32, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(lenient_i64(&value)
        .map(|n| n as i32)
        .unwrap_or(MESSAGE_NOTIFICATIONS_INHERIT))
}

fn deserialize_lenient_u64_opt<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(lenient_i64(&value).and_then(|n| u64::try_from(n).ok()))
}

fn deserialize_lenient_string_opt<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<String>, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(match value {
        Value::String(s) => Some(s),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

/// A value that is `None` when it is missing, `null`, or not what was
/// expected.
fn deserialize_lenient_opt<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = Value::deserialize(d)?;
    Ok(serde_json::from_value::<T>(value).ok())
}

/// A list whose entries that do not parse are dropped instead of failing
/// the whole payload.
fn deserialize_lenient_vec<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = Value::deserialize(d)?;
    Ok(match value {
        Value::Array(items) => items
            .into_iter()
            .filter_map(|v| serde_json::from_value::<T>(v).ok())
            .collect(),
        _ => Vec::new(),
    })
}

/// An ISO 8601 string, or a Unix time in milliseconds, or nothing.
fn deserialize_end_time<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(match &value {
        Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        Value::Number(_) => lenient_i64(&value)
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        _ => None,
    })
}

/// Channel overrides: a map keyed by channel id (the API), a list of
/// objects each carrying its `channel_id` (the gateway), or `null`.
fn deserialize_channel_overrides<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<HashMap<String, UserGuildChannelOverride>, D::Error> {
    let value = Value::deserialize(d)?;
    let mut out = HashMap::new();
    match value {
        Value::Object(map) => {
            for (id, entry) in map {
                if let Ok(o) = serde_json::from_value::<UserGuildChannelOverride>(entry) {
                    out.insert(id, o);
                }
            }
        }
        Value::Array(items) => {
            for entry in items {
                let id = match entry.get("channel_id") {
                    Some(Value::String(s)) => Some(s.clone()),
                    Some(Value::Number(n)) => Some(n.to_string()),
                    _ => None,
                };
                if let Some(id) = id
                    && let Ok(o) = serde_json::from_value::<UserGuildChannelOverride>(entry)
                {
                    out.insert(id, o);
                }
            }
        }
        _ => {}
    }
    Ok(out)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserGuildMuteConfig {
    #[serde(default, deserialize_with = "deserialize_end_time")]
    pub end_time: Option<String>,
    #[serde(default, deserialize_with = "deserialize_lenient_u64_opt")]
    pub selected_time_window: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserGuildChannelOverride {
    #[serde(default, deserialize_with = "deserialize_lenient_bool")]
    pub collapsed: bool,
    #[serde(default, deserialize_with = "deserialize_notification_level")]
    pub message_notifications: i32,
    #[serde(default, deserialize_with = "deserialize_lenient_bool")]
    pub muted: bool,
    #[serde(default, deserialize_with = "deserialize_lenient_opt")]
    pub mute_config: Option<UserGuildMuteConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserGuildSettingsResponse {
    #[serde(default, deserialize_with = "deserialize_lenient_string_opt")]
    pub guild_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_notification_level")]
    pub message_notifications: i32,
    #[serde(default, deserialize_with = "deserialize_lenient_bool")]
    pub muted: bool,
    #[serde(default, deserialize_with = "deserialize_lenient_opt")]
    pub mute_config: Option<UserGuildMuteConfig>,
    #[serde(default, deserialize_with = "deserialize_lenient_bool")]
    pub mobile_push: bool,
    #[serde(default, deserialize_with = "deserialize_lenient_bool")]
    pub suppress_everyone: bool,
    #[serde(default, deserialize_with = "deserialize_lenient_bool")]
    pub suppress_roles: bool,
    #[serde(default, deserialize_with = "deserialize_lenient_bool")]
    pub hide_muted_channels: bool,
    #[serde(default, deserialize_with = "deserialize_channel_overrides")]
    pub channel_overrides: HashMap<String, UserGuildChannelOverride>,
    #[serde(default, deserialize_with = "deserialize_lenient_i32")]
    pub version: i32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserGuildSettingsPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_notifications: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mute_config: Option<Option<UserGuildMuteConfig>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mobile_push: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppress_everyone: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppress_roles: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hide_muted_channels: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct MessageResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub author: UserPartialResponse,
    #[serde(default, rename = "type")]
    pub message_type: i32,
    #[serde(default)]
    pub tts: bool,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub timestamp: String,
    #[serde(default)]
    pub edited_timestamp: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub mention_everyone: bool,
    #[serde(default)]
    pub mentions: Vec<UserPartialResponse>,
    #[serde(default)]
    pub mention_roles: Vec<String>,
    #[serde(default)]
    pub attachments: Vec<MessageAttachmentResponse>,
    #[serde(default)]
    pub stickers: Vec<MessageStickerResponse>,
    #[serde(default)]
    pub channel_type: Option<i32>,
    #[serde(default)]
    pub embeds: Vec<MessageEmbedResponse>,
    #[serde(default)]
    pub reactions: Vec<MessageReactionResponse>,
    #[serde(default)]
    pub message_reference: Option<MessageReferenceResponse>,
    #[serde(default)]
    pub referenced_message: Option<Box<MessageResponse>>,
    #[serde(default)]
    pub member: Option<GuildMemberResponse>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct MessageEmbedResponse {
    #[serde(default, rename = "type")]
    pub embed_type: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub color: Option<i64>,
    #[serde(default)]
    pub author: Option<EmbedAuthorResponse>,
    #[serde(default)]
    pub footer: Option<EmbedFooterResponse>,
    #[serde(default)]
    pub fields: Vec<EmbedFieldResponse>,
    #[serde(default)]
    pub provider: Option<EmbedAuthorResponse>,
    #[serde(default)]
    pub image: Option<EmbedMediaResponse>,
    #[serde(default)]
    pub thumbnail: Option<EmbedMediaResponse>,
    /// GIF providers (type `gifv`) and video sites: the moving picture as
    /// WebM/MP4 or a player page. For GIFs the animation itself is the
    /// `thumbnail`.
    #[serde(default)]
    pub video: Option<EmbedMediaResponse>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct EmbedAuthorResponse {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct EmbedFooterResponse {
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct EmbedFieldResponse {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub inline: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserSettingsResponse {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub theme: String,
    #[serde(default)]
    pub locale: String,
    #[serde(default)]
    pub developer_mode: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReadyEvent {
    #[serde(default)]
    pub version: u64,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub user: UserPrivateResponse,
    #[serde(default)]
    pub guilds: Vec<GuildCreateEvent>,
    #[serde(default)]
    pub private_channels: Vec<ChannelResponse>,
    #[serde(default)]
    pub users: Vec<UserPartialResponse>,
    #[serde(default)]
    pub user_settings: Option<UserSettingsResponse>,
    #[serde(default, deserialize_with = "deserialize_lenient_vec")]
    pub user_guild_settings: Vec<UserGuildSettingsResponse>,
    #[serde(
        default,
        alias = "read_state",
        rename = "read_states",
        deserialize_with = "deserialize_lenient_vec"
    )]
    pub read_state: Vec<ReadStateResponse>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuildCreateEvent {
    #[serde(flatten)]
    pub guild: GuildResponse,
    #[serde(default)]
    pub unavailable: bool,
    #[serde(default)]
    pub channels: Vec<ChannelResponse>,
    #[serde(default)]
    pub members: Vec<GuildMemberResponse>,
    #[serde(default)]
    pub roles: Vec<GuildRoleResponse>,
    #[serde(default)]
    pub stickers: Vec<GuildStickerResponse>,
    #[serde(default)]
    pub voice_states: Vec<VoiceStateResponse>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuildDeleteEvent {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub unavailable: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelBulkUpdateEvent {
    #[serde(default)]
    pub channels: Vec<ChannelResponse>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageDeleteEvent {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub channel_id: String,
}

/// Gateway `TYPING_START` (`TypingStart.tsx` / `TypingStore.startTyping`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TypingStartEvent {
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub guild_id: Option<String>,
    #[serde(default)]
    pub member: Option<GuildMemberResponse>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VoiceStateResponse {
    #[serde(default)]
    pub guild_id: Option<String>,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub connection_id: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub member: Option<GuildMemberResponse>,
    #[serde(default)]
    pub mute: bool,
    #[serde(default)]
    pub deaf: bool,
    #[serde(default)]
    pub self_mute: bool,
    #[serde(default)]
    pub self_deaf: bool,
    #[serde(default)]
    pub self_video: bool,
    #[serde(default)]
    pub self_stream: bool,
    #[serde(default)]
    pub is_mobile: bool,
    #[serde(default)]
    pub version: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthSessionChangeEvent {
    #[serde(default)]
    pub new_token: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CallEvent {
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub ringing: Vec<String>,
    #[serde(default)]
    pub voice_states: Vec<VoiceStateResponse>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CallDeleteEvent {
    #[serde(default)]
    pub channel_id: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct EditMessageRequest {
    pub content: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateMessageRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flags: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tts: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_reference: Option<MessageReferenceRequest>,
    /// Uploads already PUT to their presigned URLs, referenced by upload key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<CreateMessageAttachment>>,
    /// The stickers sent with the message, at most three.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sticker_ids: Option<Vec<String>>,
}

/// One finished upload to reference from `CreateMessageRequest.attachments`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateMessageAttachment {
    pub id: u32,
    pub filename: String,
    pub upload_filename: String,
    pub file_size: u64,
    pub content_type: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PresignedAttachmentUploadRequestItem {
    pub id: u32,
    pub filename: String,
    pub file_size: u64,
    pub content_type: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PresignedAttachmentUploadRequest {
    pub attachments: Vec<PresignedAttachmentUploadRequestItem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PresignedUploadPart {
    #[serde(default)]
    pub part_number: u32,
    #[serde(default)]
    pub upload_url: String,
}

/// Server plan for one attachment: a single PUT for files up to 10 MB, or a
/// multipart plan (per-part URLs plus an upload_id to complete) above that.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PresignedAttachmentUploadResponseItem {
    #[serde(default)]
    pub id: u32,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub upload_filename: String,
    #[serde(default)]
    pub file_size: u64,
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub upload_mode: String,
    #[serde(default)]
    pub upload_url: Option<String>,
    #[serde(default)]
    pub upload_id: Option<String>,
    #[serde(default)]
    pub part_size: Option<u64>,
    #[serde(default)]
    pub parts: Vec<PresignedUploadPart>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PresignedAttachmentUploadResponse {
    #[serde(default)]
    pub attachments: Vec<PresignedAttachmentUploadResponseItem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompleteMultipartUploadItem {
    pub upload_filename: String,
    pub upload_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompleteMultipartAttachmentUploadRequest {
    pub uploads: Vec<CompleteMultipartUploadItem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub around: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GatewayHelloPayload {
    #[serde(default)]
    pub heartbeat_interval: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GatewayPayload {
    #[serde(default)]
    pub op: u8,
    #[serde(default)]
    pub d: Value,
    #[serde(default)]
    pub s: Option<u64>,
    #[serde(default)]
    pub t: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GatewayIdentifyPayload {
    pub token: String,
    pub properties: GatewayIdentifyProperties,
    pub flags: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_guild_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GatewayIdentifyProperties {
    pub os: String,
    pub browser: String,
    pub device: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GatewayResumePayload {
    pub token: String,
    pub session_id: String,
    pub seq: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageReactionAddEvent {
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub guild_id: Option<String>,
    #[serde(default)]
    pub emoji: ReactionEmojiResponse,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageReactionRemoveEvent {
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub guild_id: Option<String>,
    #[serde(default)]
    pub emoji: ReactionEmojiResponse,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageAckEvent {
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub mention_count: u64,
}

pub fn snowflake_sort_key(value: &str) -> u128 {
    value.parse::<u128>().unwrap_or_default()
}

pub fn merge_user_cache(
    cache: &mut HashMap<Snowflake, UserPartialResponse>,
    users: impl IntoIterator<Item = UserPartialResponse>,
) {
    for user in users {
        if !user.id.is_empty() {
            cache.insert(user.id.clone(), user);
        }
    }
}

/// VOICE_SERVER_UPDATE: the grant this session presents to the media
/// server. Fluxer carries voice over LiveKit — there is no second voice
/// websocket and no voice opcode set — so a client opens a LiveKit
/// connection to `endpoint` and presents `token` there.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct VoiceServerUpdateEvent {
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub connection_id: String,
    #[serde(default)]
    pub channel_id: String,
    /// There for a community's voice channel and absent for a call, so
    /// this is how the scope is read.
    #[serde(default)]
    pub guild_id: Option<String>,
    /// There only when the channel is end-to-end encrypted.
    #[serde(default)]
    pub e2ee_key: Option<String>,
}

/// VOICE_STATE_ACK: what the server made of an opcode 4, echoing back
/// the `mutation_id` the client sent.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct VoiceStateAckEvent {
    #[serde(default)]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub channel_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_event_accepts_read_states_payload() {
        let ready: ReadyEvent = serde_json::from_value(serde_json::json!({
            "session_id": "sess",
            "read_states": [
                {
                    "id": "chan-1",
                    "last_message_id": "42",
                    "mention_count": 3
                }
            ]
        }))
        .expect("READY payload should deserialize");

        assert_eq!(ready.read_state.len(), 1);
        assert_eq!(ready.read_state[0].id, "chan-1");
        assert_eq!(ready.read_state[0].last_message_id.as_deref(), Some("42"));
        assert_eq!(ready.read_state[0].mention_count, 3);
    }

    #[test]
    fn gif_embed_carries_its_video_and_thumbnail() {
        let embed: MessageEmbedResponse = serde_json::from_value(serde_json::json!({
            "type": "gifv",
            "url": "https://klipy.com/gifs/linux-kernel-tux",
            "provider": {"name": "KLIPY", "url": "https://klipy.com/"},
            "thumbnail": {
                "url": "https://static.klipy.com/ii/9d/52/B9ynyBGO.webp",
                "proxy_url": "https://fluxerusercontent.com/external/k/https/static.klipy.com/ii/9d/52/B9ynyBGO.webp",
                "width": 312, "height": 312, "content_type": "image/webp", "flags": 32
            },
            "video": {
                "url": "https://static.klipy.com/ii/9d/52/DkIvrEVx48Lh.webm",
                "proxy_url": "https://fluxerusercontent.com/external/Z/https/static.klipy.com/ii/9d/52/DkIvrEVx48Lh.webm",
                "width": 312, "height": 312, "duration": 2, "content_type": "video/webm", "flags": 0
            },
            "image": null,
            "title": null
        }))
        .expect("gifv embed should deserialize");

        assert_eq!(embed.embed_type, "gifv");
        assert!(embed.image.is_none());
        assert!(
            embed
                .thumbnail
                .as_ref()
                .and_then(|m| m.proxy_url.as_deref())
                .is_some_and(|u| u.ends_with(".webp"))
        );
        assert!(
            embed
                .video
                .as_ref()
                .and_then(|m| m.url.as_deref())
                .is_some_and(|u| u.ends_with(".webm"))
        );
    }
}

#[cfg(test)]
mod user_guild_settings_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_api_answer_to_an_update_parses_with_no_overrides() {
        // What PATCH /users/@me/guilds/{id}/settings answers for a
        // community without per-channel overrides.
        let settings: UserGuildSettingsResponse = serde_json::from_value(json!({
            "guild_id": "1471251973335237061",
            "message_notifications": 1,
            "muted": false,
            "mute_config": null,
            "mobile_push": true,
            "suppress_everyone": false,
            "suppress_roles": true,
            "hide_muted_channels": false,
            "channel_overrides": null,
            "unread_badges": null,
            "version": 2
        }))
        .expect("the update answer should parse");
        assert_eq!(settings.guild_id.as_deref(), Some("1471251973335237061"));
        assert_eq!(
            settings.message_notifications,
            MESSAGE_NOTIFICATIONS_ONLY_MENTIONS
        );
        assert!(settings.suppress_roles);
        assert!(settings.channel_overrides.is_empty());
        assert_eq!(settings.version, 2);
    }

    #[test]
    fn the_gateway_spelling_parses_too() {
        let settings: UserGuildSettingsResponse = serde_json::from_value(json!({
            "guild_id": 1471251973335237061u64,
            "message_notifications": null,
            "muted": true,
            "mute_config": {"end_time": 1_800_000_000_000u64, "selected_time_window": "900000"},
            "mobile_push": null,
            "channel_overrides": [
                {"channel_id": "7", "muted": true, "message_notifications": 2, "collapsed": false},
                {"muted": true}
            ],
            "version": "3"
        }))
        .expect("the gateway spelling should parse");
        assert_eq!(settings.guild_id.as_deref(), Some("1471251973335237061"));
        assert_eq!(
            settings.message_notifications,
            MESSAGE_NOTIFICATIONS_INHERIT
        );
        assert!(settings.muted);
        let mute = settings.mute_config.expect("mute config");
        assert_eq!(mute.selected_time_window, Some(900_000));
        assert!(mute.end_time.is_some_and(|t| t.starts_with("2027-01-15T")));
        assert!(!settings.mobile_push);
        assert_eq!(settings.channel_overrides.len(), 1);
        let over = &settings.channel_overrides["7"];
        assert!(over.muted);
        assert_eq!(
            over.message_notifications,
            MESSAGE_NOTIFICATIONS_NO_MESSAGES
        );
        assert_eq!(settings.version, 3);
    }

    #[test]
    fn a_ready_payload_survives_a_settings_entry_it_cannot_read() {
        let ready: ReadyEvent = serde_json::from_value(json!({
            "session_id": "s",
            "user": {"id": "me"},
            "user_guild_settings": [
                "not even an object",
                {"guild_id": "2", "muted": true, "channel_overrides": null}
            ],
            "read_states": [{"id": "9", "mention_count": "nope"}, {"id": "10", "mention_count": 2}]
        }))
        .expect("READY should parse");
        // the first entry is not a settings object at all; it is dropped
        assert_eq!(ready.user_guild_settings.len(), 1);
        assert_eq!(ready.user_guild_settings[0].guild_id.as_deref(), Some("2"));
        assert_eq!(ready.read_state.len(), 1);
        assert_eq!(ready.read_state[0].mention_count, 2);
    }

    #[test]
    fn the_patch_sends_only_what_changed_and_null_to_clear_a_mute() {
        let clear = UserGuildSettingsPatch {
            muted: Some(false),
            mute_config: Some(None),
            ..UserGuildSettingsPatch::default()
        };
        assert_eq!(
            serde_json::to_value(&clear).unwrap(),
            json!({"muted": false, "mute_config": null})
        );
        let timed = UserGuildSettingsPatch {
            muted: Some(true),
            mute_config: Some(Some(UserGuildMuteConfig {
                end_time: Some("2026-09-07T12:00:00.000Z".into()),
                selected_time_window: Some(900_000),
            })),
            ..UserGuildSettingsPatch::default()
        };
        assert_eq!(
            serde_json::to_value(&timed).unwrap(),
            json!({
                "muted": true,
                "mute_config": {"end_time": "2026-09-07T12:00:00.000Z", "selected_time_window": 900000}
            })
        );
    }
}
