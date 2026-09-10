use crate::api::types::{
    ChannelResponse, CompleteMultipartAttachmentUploadRequest, CompleteMultipartUploadItem,
    CreateMessageAttachment, CreateMessageRequest, DiscoveryGuildListResponse, EditMessageRequest,
    GatewayBotResponse, GuildResponse, HandoffInitiateResponse, HandoffStatusResponse,
    InviteResponse, MessageQuery, MessageResponse, PresignedAttachmentUploadRequest,
    PresignedAttachmentUploadRequestItem, PresignedAttachmentUploadResponse,
    UserGuildSettingsPatch, UserGuildSettingsResponse, UserPrivateResponse, UserSettingsResponse,
    WellKnownFluxerResponse,
};
use crate::media::StagedAttachment;
use anyhow::{Context, Result, anyhow, bail};
use reqwest::{Method, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use thiserror::Error;
use tokio::time::{Duration, sleep};
use urlencoding;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("{status} {message}")]
    Response {
        status: StatusCode,
        code: Option<String>,
        message: String,
        body: Value,
    },
}

/// How long one page of a member list may take before it is given up on.
const MEMBERS_PAGE_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Debug, Error)]
#[error("no answer within {0} seconds")]
pub struct MembersTimeout(pub u64);

/// Why a member list could not be fetched, as far as the client can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MembersFailure {
    /// The server did not answer, or not in time (a 5xx from its gateway,
    /// a timeout, no connection): worth trying again later.
    Unavailable,
    /// The community does not let this user list its members: not worth
    /// trying again.
    Forbidden,
    Other,
}

pub fn members_failure(err: &anyhow::Error) -> MembersFailure {
    if let Some(ApiError::Response { status, .. }) = err.downcast_ref::<ApiError>() {
        return match *status {
            StatusCode::FORBIDDEN => MembersFailure::Forbidden,
            s if s.is_server_error() || s == StatusCode::REQUEST_TIMEOUT => {
                MembersFailure::Unavailable
            }
            _ => MembersFailure::Other,
        };
    }
    if err.downcast_ref::<MembersTimeout>().is_some()
        || err.chain().any(|cause| {
            cause
                .downcast_ref::<reqwest::Error>()
                .is_some_and(|e| e.is_timeout() || e.is_connect())
        })
    {
        return MembersFailure::Unavailable;
    }
    MembersFailure::Other
}

#[derive(Debug, Clone)]
pub struct FluxerHttpClient {
    inner: reqwest::Client,
    base_url: String,
    token: Option<String>,
}

impl FluxerHttpClient {
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        let (os_token, platform_token) = match std::env::consts::OS {
            "linux" => ("Linux", "X11"),
            "macos" => ("Mac OS X", "Macintosh"),
            "windows" => ("Windows NT 10.0", "Windows"),
            other => (other, other),
        };
        let arch = std::env::consts::ARCH;
        let ua = format!(
            "Mozilla/5.0 ({platform_token}; {os_token}; {arch}) FluxerTUI/{}",
            env!("CARGO_PKG_VERSION")
        );

        // isreali GPT was here... Beep Boop. (joke)\

        Ok(Self {
            inner: reqwest::Client::builder()
                .user_agent(ua)
                .build()
                .context("failed to build HTTP client")?,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token: None,
        })
    }

    pub fn with_token(&self, token: impl Into<String>) -> Self {
        let mut client = self.clone();
        client.token = Some(token.into());
        client
    }

    pub async fn discover(&self) -> Result<WellKnownFluxerResponse> {
        self.send_json::<(), (), WellKnownFluxerResponse>(
            Method::GET,
            "/.well-known/fluxer",
            None::<&()>,
            None::<&()>,
            true,
        )
        .await
    }

    pub async fn gateway_info(&self) -> Result<GatewayBotResponse> {
        self.send_json::<(), (), GatewayBotResponse>(
            Method::GET,
            "/gateway/bot",
            None::<&()>,
            None::<&()>,
            true,
        )
        .await
    }

    pub async fn current_user(&self) -> Result<UserPrivateResponse> {
        self.send_json::<(), (), UserPrivateResponse>(
            Method::GET,
            "/users/@me",
            None::<&()>,
            None::<&()>,
            false,
        )
        .await
    }

    pub async fn current_user_settings(&self) -> Result<UserSettingsResponse> {
        self.send_json::<(), (), UserSettingsResponse>(
            Method::GET,
            "/users/@me/settings",
            None::<&()>,
            None::<&()>,
            false,
        )
        .await
    }

    pub async fn update_user_guild_settings(
        &self,
        guild_id: Option<&str>,
        body: &UserGuildSettingsPatch,
    ) -> Result<UserGuildSettingsResponse> {
        let path = match guild_id {
            Some(guild_id) => format!("/users/@me/guilds/{guild_id}/settings"),
            None => "/users/@me/guilds/@me/settings".to_string(),
        };
        self.send_json::<(), UserGuildSettingsPatch, UserGuildSettingsResponse>(
            Method::PATCH,
            &path,
            None::<&()>,
            Some(body),
            false,
        )
        .await
    }

    pub async fn guilds(&self) -> Result<Vec<GuildResponse>> {
        self.send_json::<(), (), Vec<GuildResponse>>(
            Method::GET,
            "/users/@me/guilds",
            None::<&()>,
            None::<&()>,
            false,
        )
        .await
    }

    pub async fn private_channels(&self) -> Result<Vec<ChannelResponse>> {
        self.send_json::<(), (), Vec<ChannelResponse>>(
            Method::GET,
            "/users/@me/channels",
            None::<&()>,
            None::<&()>,
            false,
        )
        .await
    }

    pub async fn guild_channels(&self, guild_id: &str) -> Result<Vec<ChannelResponse>> {
        self.send_json::<(), (), Vec<ChannelResponse>>(
            Method::GET,
            &format!("/guilds/{guild_id}/channels"),
            None::<&()>,
            None::<&()>,
            false,
        )
        .await
    }

    /// Every member of a community, page by page. The pages that arrived
    /// before a page failed come back together with the error that stopped
    /// the fetch: a list that is only partly there is still worth having.
    pub async fn guild_members(
        &self,
        guild_id: &str,
    ) -> (
        Vec<crate::api::types::GuildMemberResponse>,
        Option<anyhow::Error>,
    ) {
        #[derive(Serialize)]
        struct MembersQuery<'a> {
            limit: u32,
            #[serde(skip_serializing_if = "Option::is_none")]
            after: Option<&'a str>,
        }

        let mut all = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let query = MembersQuery {
                limit: 1000,
                after: after.as_deref(),
            };
            let path = format!("/guilds/{guild_id}/members");
            let page = self
                .send_json::<MembersQuery, (), Vec<crate::api::types::GuildMemberResponse>>(
                    Method::GET,
                    &path,
                    Some(&query),
                    None::<&()>,
                    false,
                );
            let batch = match tokio::time::timeout(MEMBERS_PAGE_TIMEOUT, page).await {
                Ok(Ok(batch)) => batch,
                Ok(Err(err)) => return (all, Some(err)),
                Err(_) => {
                    return (
                        all,
                        Some(MembersTimeout(MEMBERS_PAGE_TIMEOUT.as_secs()).into()),
                    );
                }
            };
            let n = batch.len();
            if n == 0 {
                break;
            }
            let last_id = batch.last().unwrap().user.id.clone();
            all.extend(batch);
            if n < 1000 {
                break;
            }
            sleep(Duration::from_millis(400)).await;
            after = Some(last_id);
        }
        (all, None)
    }

    pub async fn guild_emojis(
        &self,
        guild_id: &str,
    ) -> Result<Vec<crate::api::types::GuildEmojiResponse>> {
        self.send_json::<(), (), Vec<crate::api::types::GuildEmojiResponse>>(
            Method::GET,
            &format!("/guilds/{guild_id}/emojis"),
            None::<&()>,
            None::<&()>,
            false,
        )
        .await
    }

    pub async fn guild_stickers(
        &self,
        guild_id: &str,
    ) -> Result<Vec<crate::api::types::GuildStickerResponse>> {
        self.send_json::<(), (), Vec<crate::api::types::GuildStickerResponse>>(
            Method::GET,
            &format!("/guilds/{guild_id}/stickers"),
            None::<&()>,
            None::<&()>,
            false,
        )
        .await
    }

    pub async fn guild_roles(
        &self,
        guild_id: &str,
    ) -> Result<Vec<crate::api::types::GuildRoleResponse>> {
        self.send_json::<(), (), Vec<crate::api::types::GuildRoleResponse>>(
            Method::GET,
            &format!("/guilds/{guild_id}/roles"),
            None::<&()>,
            None::<&()>,
            false,
        )
        .await
    }

    /// A user's profile as the web app's popup shows it; with `guild_id`
    /// also their member data and guild profile there.
    pub async fn user_profile(
        &self,
        user_id: &str,
        guild_id: Option<&str>,
    ) -> Result<crate::api::types::UserProfileResponse> {
        let mut query: Vec<(&str, &str)> = vec![
            ("with_mutual_guilds", "true"),
            ("with_mutual_friends", "true"),
        ];
        if let Some(gid) = guild_id {
            query.push(("guild_id", gid));
        }
        self.send_json::<[(&str, &str)], (), crate::api::types::UserProfileResponse>(
            Method::GET,
            &format!("/users/{user_id}/profile"),
            Some(query.as_slice()),
            None::<&()>,
            false,
        )
        .await
    }

    pub async fn patch_current_guild_member_nick(
        &self,
        guild_id: &str,
        nick: Option<&str>,
    ) -> Result<crate::api::types::GuildMemberResponse> {
        let body = match nick {
            Some(s) => serde_json::json!({ "nick": s }),
            None => serde_json::json!({ "nick": serde_json::Value::Null }),
        };
        self.send_json::<(), serde_json::Value, crate::api::types::GuildMemberResponse>(
            Method::PATCH,
            &format!("/guilds/{guild_id}/members/@me"),
            None::<&()>,
            Some(&body),
            false,
        )
        .await
    }

    pub async fn channel_messages(
        &self,
        channel_id: &str,
        query: &MessageQuery,
    ) -> Result<Vec<MessageResponse>> {
        self.send_json::<MessageQuery, (), Vec<MessageResponse>>(
            Method::GET,
            &format!("/channels/{channel_id}/messages"),
            Some(query),
            None::<&()>,
            false,
        )
        .await
    }

    pub async fn send_message(
        &self,
        channel_id: &str,
        body: &CreateMessageRequest,
    ) -> Result<MessageResponse> {
        self.send_json(
            Method::POST,
            &format!("/channels/{channel_id}/messages"),
            None::<&()>,
            Some(body),
            false,
        )
        .await
    }

    /// Plan uploads, PUT the bytes to the presigned URLs the server hands
    /// back (completing multipart plans when it chose that), and return the
    /// references to put on the message.
    pub async fn upload_attachments(
        &self,
        channel_id: &str,
        staged: &[StagedAttachment],
    ) -> Result<Vec<CreateMessageAttachment>> {
        let request = PresignedAttachmentUploadRequest {
            attachments: staged
                .iter()
                .enumerate()
                .map(|(i, a)| PresignedAttachmentUploadRequestItem {
                    id: i as u32,
                    filename: a.filename.clone(),
                    file_size: a.bytes.len() as u64,
                    content_type: a.content_type.clone(),
                })
                .collect(),
        };
        let plan: PresignedAttachmentUploadResponse = self
            .send_json(
                Method::POST,
                &format!("/channels/{channel_id}/attachments"),
                None::<&()>,
                Some(&request),
                false,
            )
            .await
            .context("attachment upload was refused")?;

        let mut done = Vec::with_capacity(staged.len());
        let mut to_complete = Vec::new();
        for item in plan.attachments {
            let Some(src) = staged.get(item.id as usize) else {
                bail!(
                    "server returned an upload plan for an unknown attachment id {}",
                    item.id
                );
            };
            let content_type = if item.content_type.is_empty() {
                src.content_type.clone()
            } else {
                item.content_type.clone()
            };
            match item.upload_mode.as_str() {
                "singlepart" => {
                    let url = item
                        .upload_url
                        .as_deref()
                        .filter(|u| !u.is_empty())
                        .ok_or_else(|| anyhow!("singlepart plan without an upload_url"))?;
                    self.put_presigned(url, &content_type, src.bytes.clone())
                        .await
                        .with_context(|| format!("uploading {}", src.filename))?;
                }
                "multipart" => {
                    let part_size = item
                        .part_size
                        .filter(|n| *n > 0)
                        .ok_or_else(|| anyhow!("multipart plan without a part_size"))?
                        as usize;
                    let upload_id = item
                        .upload_id
                        .clone()
                        .filter(|u| !u.is_empty())
                        .ok_or_else(|| anyhow!("multipart plan without an upload_id"))?;
                    let chunks: Vec<&[u8]> = src.bytes.chunks(part_size).collect();
                    for part in &item.parts {
                        let idx = part.part_number.saturating_sub(1) as usize;
                        let Some(chunk) = chunks.get(idx) else {
                            bail!(
                                "multipart plan for {} names part {} but the file only has {} parts",
                                src.filename,
                                part.part_number,
                                chunks.len()
                            );
                        };
                        self.put_presigned(&part.upload_url, &content_type, chunk.to_vec())
                            .await
                            .with_context(|| {
                                format!("uploading {} part {}", src.filename, part.part_number)
                            })?;
                    }
                    to_complete.push(CompleteMultipartUploadItem {
                        upload_filename: item.upload_filename.clone(),
                        upload_id,
                    });
                }
                other => bail!("unknown upload_mode {other:?} for {}", src.filename),
            }
            done.push(CreateMessageAttachment {
                id: item.id,
                filename: src.filename.clone(),
                upload_filename: item.upload_filename,
                file_size: src.bytes.len() as u64,
                content_type,
            });
        }

        if !to_complete.is_empty() {
            let body = CompleteMultipartAttachmentUploadRequest {
                uploads: to_complete,
            };
            let _: Value = self
                .send_json(
                    Method::POST,
                    &format!("/channels/{channel_id}/attachments/complete"),
                    None::<&()>,
                    Some(&body),
                    false,
                )
                .await
                .context("completing multipart upload")?;
        }
        Ok(done)
    }

    /// PUT raw bytes to a presigned storage URL. No Fluxer auth header: the
    /// signature in the URL is the credential, and extra headers would break it.
    async fn put_presigned(&self, url: &str, content_type: &str, bytes: Vec<u8>) -> Result<()> {
        let response = self
            .inner
            .put(url)
            .header("Content-Type", content_type)
            .body(bytes)
            .send()
            .await
            .context("storage PUT failed")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let body = body.trim();
            bail!(
                "storage PUT returned {status}{}",
                if body.is_empty() {
                    String::new()
                } else {
                    format!(": {}", body.chars().take(200).collect::<String>())
                }
            );
        }
        Ok(())
    }

    pub async fn edit_message(
        &self,
        channel_id: &str,
        message_id: &str,
        content: &str,
    ) -> Result<MessageResponse> {
        let body = EditMessageRequest {
            content: content.to_string(),
        };
        self.send_json(
            Method::PATCH,
            &format!("/channels/{channel_id}/messages/{message_id}"),
            None::<&()>,
            Some(&body),
            false,
        )
        .await
    }

    pub async fn delete_message(&self, channel_id: &str, message_id: &str) -> Result<()> {
        let resp = self
            .inner
            .request(
                Method::DELETE,
                self.url(&format!("/channels/{channel_id}/messages/{message_id}")),
            )
            .header("X-Fluxer-Platform", "desktop")
            .header("Authorization", self.token.as_deref().unwrap_or(""))
            .send()
            .await
            .context("failed to delete message")?;
        if !resp.status().is_success() && resp.status() != StatusCode::NO_CONTENT {
            bail!("delete message failed: {}", resp.status());
        }
        Ok(())
    }

    pub async fn ack_message(&self, channel_id: &str, message_id: &str) -> Result<()> {
        let resp = self
            .inner
            .request(
                Method::POST,
                self.url(&format!("/channels/{channel_id}/messages/{message_id}/ack")),
            )
            .header("X-Fluxer-Platform", "desktop")
            .header("Authorization", self.token.as_deref().unwrap_or(""))
            .send()
            .await
            .context("failed to ack message")?;
        if !resp.status().is_success() && resp.status() != StatusCode::NO_CONTENT {
            bail!("ack failed: {}", resp.status());
        }
        Ok(())
    }

    /// Tell the channel the user is typing; the server shows it to the
    /// others for about ten seconds.
    pub async fn start_typing(&self, channel_id: &str) -> Result<()> {
        let resp = self
            .inner
            .request(
                Method::POST,
                self.url(&format!("/channels/{channel_id}/typing")),
            )
            .header("X-Fluxer-Platform", "desktop")
            .header("Authorization", self.token.as_deref().unwrap_or(""))
            .send()
            .await
            .context("failed to send typing")?;
        if !resp.status().is_success() && resp.status() != StatusCode::NO_CONTENT {
            bail!("typing failed: {}", resp.status());
        }
        Ok(())
    }

    /// The messages that mentioned the user, newest first, as the web
    /// client's inbox lists them: by name, by role and with @everyone,
    /// in communities and direct messages alike, up to `limit` (100).
    pub async fn recent_mentions(&self, limit: u32) -> Result<Vec<MessageResponse>> {
        #[derive(Serialize)]
        struct Query {
            limit: u32,
        }
        self.send_json::<Query, (), Vec<MessageResponse>>(
            Method::GET,
            "/users/@me/mentions",
            Some(&Query { limit }),
            None::<&()>,
            false,
        )
        .await
    }

    /// Take one message off the user's mention list.
    pub async fn dismiss_mention(&self, message_id: &str) -> Result<()> {
        let resp = self
            .inner
            .request(
                Method::DELETE,
                self.url(&format!("/users/@me/mentions/{message_id}")),
            )
            .header("X-Fluxer-Platform", "desktop")
            .header("Authorization", self.token.as_deref().unwrap_or(""))
            .send()
            .await
            .context("failed to dismiss mention")?;
        if !resp.status().is_success() && resp.status() != StatusCode::NO_CONTENT {
            bail!("dismiss mention failed: {}", resp.status());
        }
        Ok(())
    }

    /// Take several messages off the user's mention list at once (the
    /// server takes up to 100 per call).
    pub async fn dismiss_mentions(&self, message_ids: &[String]) -> Result<()> {
        #[derive(Serialize)]
        struct Body<'a> {
            message_ids: &'a [String],
        }
        for chunk in message_ids.chunks(100) {
            let resp = self
                .inner
                .request(Method::POST, self.url("/users/@me/mentions/read"))
                .header("X-Fluxer-Platform", "desktop")
                .header("Authorization", self.token.as_deref().unwrap_or(""))
                .json(&Body { message_ids: chunk })
                .send()
                .await
                .context("failed to dismiss mentions")?;
            if !resp.status().is_success() && resp.status() != StatusCode::NO_CONTENT {
                bail!("dismiss mentions failed: {}", resp.status());
            }
        }
        Ok(())
    }

    pub async fn add_reaction(
        &self,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> Result<()> {
        let encoded = urlencoding::encode(emoji);
        let resp = self
            .inner
            .request(
                Method::PUT,
                self.url(&format!(
                    "/channels/{channel_id}/messages/{message_id}/reactions/{encoded}/@me"
                )),
            )
            .header("X-Fluxer-Platform", "desktop")
            .header("Authorization", self.token.as_deref().unwrap_or(""))
            .send()
            .await
            .context("failed to add reaction")?;
        if !resp.status().is_success() && resp.status() != StatusCode::NO_CONTENT {
            bail!("add reaction failed: {}", resp.status());
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn remove_reaction(
        &self,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> Result<()> {
        let encoded = urlencoding::encode(emoji);
        let resp = self
            .inner
            .request(
                Method::DELETE,
                self.url(&format!(
                    "/channels/{channel_id}/messages/{message_id}/reactions/{encoded}/@me"
                )),
            )
            .header("X-Fluxer-Platform", "desktop")
            .header("Authorization", self.token.as_deref().unwrap_or(""))
            .send()
            .await
            .context("failed to remove reaction")?;
        if !resp.status().is_success() && resp.status() != StatusCode::NO_CONTENT {
            bail!("remove reaction failed: {}", resp.status());
        }
        Ok(())
    }

    /// What an invite leads to, without taking it.
    pub async fn invite_info(&self, code: &str) -> Result<InviteResponse> {
        self.send_json::<(), (), InviteResponse>(
            Method::GET,
            &format!("/invites/{code}"),
            None::<&()>,
            None::<&()>,
            false,
        )
        .await
    }

    /// Take an invite: join the community, or the group conversation.
    pub async fn accept_invite(&self, code: &str) -> Result<InviteResponse> {
        self.send_json::<(), serde_json::Value, InviteResponse>(
            Method::POST,
            &format!("/invites/{code}"),
            None::<&()>,
            Some(&serde_json::json!({})),
            false,
        )
        .await
    }

    /// Make an invite to a channel. `max_age` is in seconds and
    /// `max_uses` a count, both zero for "no limit".
    pub async fn create_invite(
        &self,
        channel_id: &str,
        max_age: u32,
        max_uses: u32,
    ) -> Result<InviteResponse> {
        #[derive(Serialize)]
        struct Body {
            max_age: u32,
            max_uses: u32,
        }
        self.send_json::<(), Body, InviteResponse>(
            Method::POST,
            &format!("/channels/{channel_id}/invites"),
            None::<&()>,
            Some(&Body { max_age, max_uses }),
            false,
        )
        .await
    }

    /// Every invite of a community that the reader may see. Needs Manage
    /// Guild.
    pub async fn guild_invites(&self, guild_id: &str) -> Result<Vec<InviteResponse>> {
        self.send_json::<(), (), Vec<InviteResponse>>(
            Method::GET,
            &format!("/guilds/{guild_id}/invites"),
            None::<&()>,
            None::<&()>,
            false,
        )
        .await
    }

    pub async fn delete_invite(&self, code: &str) -> Result<()> {
        self.send_empty::<()>(
            Method::DELETE,
            &format!("/invites/{code}"),
            None,
            "revoke the invite",
        )
        .await
    }

    pub async fn create_guild(&self, name: &str) -> Result<GuildResponse> {
        #[derive(Serialize)]
        struct Body<'a> {
            name: &'a str,
        }
        self.send_json::<(), Body, GuildResponse>(
            Method::POST,
            "/guilds",
            None::<&()>,
            Some(&Body { name }),
            false,
        )
        .await
    }

    /// Leave a community. The reader cannot leave one they own; the
    /// server says so.
    pub async fn leave_guild(&self, guild_id: &str) -> Result<()> {
        self.send_empty::<()>(
            Method::DELETE,
            &format!("/users/@me/guilds/{guild_id}"),
            None,
            "leave the community",
        )
        .await
    }

    /// Search the discovery directory.
    pub async fn discover_guilds(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<DiscoveryGuildListResponse> {
        #[derive(Serialize)]
        struct Query<'a> {
            #[serde(skip_serializing_if = "str::is_empty")]
            query: &'a str,
            limit: u32,
        }
        self.send_json::<Query, (), DiscoveryGuildListResponse>(
            Method::GET,
            "/discovery/guilds",
            Some(&Query {
                query,
                limit: limit.clamp(1, 48),
            }),
            None::<&()>,
            false,
        )
        .await
    }

    /// Join a community straight from the directory, without an invite.
    pub async fn join_discoverable_guild(&self, guild_id: &str) -> Result<()> {
        self.send_empty(
            Method::POST,
            &format!("/discovery/guilds/{guild_id}/join"),
            Some(&serde_json::json!({})),
            "join the community",
        )
        .await
    }

    /// A call whose answer is either 204 or nothing worth reading.
    async fn send_empty<B>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
        what: &str,
    ) -> Result<()>
    where
        B: Serialize + ?Sized,
    {
        let mut builder = self
            .inner
            .request(method, self.url(path))
            .header("X-Fluxer-Platform", "desktop")
            .header("Authorization", self.token.as_deref().unwrap_or(""));
        if let Some(body) = body {
            builder = builder.json(body);
        }
        let resp = builder
            .send()
            .await
            .with_context(|| format!("failed to {what}"))?;
        let status = resp.status();
        if !status.is_success() && status != StatusCode::NO_CONTENT {
            let detail = resp.text().await.unwrap_or_default();
            let detail = detail.chars().take(200).collect::<String>();
            crate::debug::log("http", format!("{what} failed: {status}"));
            if detail.is_empty() {
                bail!("{what} failed: {status}");
            }
            bail!("{what} failed: {status} {detail}");
        }
        Ok(())
    }

    pub async fn handoff_initiate(&self) -> Result<HandoffInitiateResponse> {
        self.send_json::<(), (), HandoffInitiateResponse>(
            Method::POST,
            "/auth/handoff/initiate",
            None::<&()>,
            None::<&()>,
            true,
        )
        .await
    }

    pub async fn handoff_status(
        &self,
        code: &str,
        poll_secret: Option<&str>,
    ) -> Result<HandoffStatusResponse> {
        let path = format!("/auth/handoff/{code}/status");
        match poll_secret {
            // The API only hands out the token when the poll secret from
            // `handoff_initiate` is presented in a POST body; a GET without
            // it stays "pending" forever and counts as a failed attempt.
            Some(secret) => {
                let body = serde_json::json!({ "poll_secret": secret });
                self.send_json::<(), Value, HandoffStatusResponse>(
                    Method::POST,
                    &path,
                    None::<&()>,
                    Some(&body),
                    true,
                )
                .await
            }
            None => {
                self.send_json::<(), (), HandoffStatusResponse>(
                    Method::GET,
                    &path,
                    None::<&()>,
                    None::<&()>,
                    true,
                )
                .await
            }
        }
    }

    async fn send_json<Q, B, T>(
        &self,
        method: Method,
        path: &str,
        query: Option<&Q>,
        body: Option<&B>,
        skip_auth: bool,
    ) -> Result<T>
    where
        Q: Serialize + ?Sized,
        B: Serialize + ?Sized,
        T: DeserializeOwned,
    {
        let started = std::time::Instant::now();
        let method_name = method.to_string();
        let mut builder = self
            .inner
            .request(method, self.url(path))
            .header("X-Fluxer-Platform", "desktop");

        if !skip_auth {
            let token = self
                .token
                .as_deref()
                .ok_or_else(|| anyhow!("authentication token is required for {path}"))?;
            builder = builder.header("Authorization", token);
        }

        if let Some(query) = query {
            builder = builder.query(query);
        }

        if let Some(body) = body {
            builder = builder.json(body);
        }

        let response = match builder.send().await {
            Ok(response) => response,
            Err(err) => {
                crate::debug::log(
                    "http",
                    format!(
                        "{method_name} {path} failed after {} ms: {err}",
                        started.elapsed().as_millis()
                    ),
                );
                return Err(err).with_context(|| format!("request failed for {path}"));
            }
        };

        let status = response.status();
        // which way the request went matters when the server counts by
        // address: a browser over IPv6 and a client over IPv4 are two
        // addresses to it
        let family = match response.remote_addr() {
            Some(addr) if addr.is_ipv6() => " over IPv6",
            Some(_) => " over IPv4",
            None => "",
        };
        crate::debug::log(
            "http",
            format!(
                "{method_name} {path} {} in {} ms{family}",
                status.as_u16(),
                started.elapsed().as_millis()
            ),
        );
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let json = serde_json::from_str::<Value>(&body).unwrap_or(Value::Null);
            let code = json
                .get("code")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let message = json
                .get("message")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| {
                    if body.is_empty() {
                        format!("request to {path} failed")
                    } else {
                        body.clone()
                    }
                });
            // the API's own words for it, and the shape of the rest
            crate::debug::log(
                "http",
                format!(
                    "{method_name} {path}: {} {}: {}",
                    status.as_u16(),
                    code.as_deref().unwrap_or("-"),
                    crate::debug::shape(&json)
                ),
            );
            return Err(ApiError::Response {
                status,
                code,
                message,
                body: json,
            }
            .into());
        }

        if status == StatusCode::NO_CONTENT {
            bail!("unexpected empty response for {path}");
        }

        match response.json::<T>().await {
            Ok(value) => Ok(value),
            Err(err) => {
                crate::debug::log(
                    "http",
                    format!("{method_name} {path}: the answer could not be read: {err}"),
                );
                Err(err).with_context(|| format!("failed to decode JSON for {path}"))
            }
        }
    }

    pub async fn fetch_url_bytes(&self, url_or_path: &str) -> Result<Vec<u8>> {
        let target = self.url(url_or_path);
        let mut req = self
            .inner
            .get(&target)
            .header("X-Fluxer-Platform", "desktop");
        if let Some(token) = self.token.as_deref() {
            if !token.is_empty() {
                req = req.header("Authorization", token);
            }
        }
        let response = req
            .send()
            .await
            .with_context(|| format!("request failed for {target}"))?;
        let status = response.status();
        if !status.is_success() {
            bail!("fetch failed: {status} ({target})");
        }
        let bytes = response.bytes().await.context("read response body")?;
        Ok(bytes.to_vec())
    }

    /// GET a public asset (media proxy, CDN) without the Fluxer auth header.
    pub async fn fetch_public_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let response = self
            .inner
            .get(url)
            .send()
            .await
            .with_context(|| format!("request failed for {url}"))?;
        let status = response.status();
        if !status.is_success() {
            bail!("fetch failed: {status} ({url})");
        }
        Ok(response
            .bytes()
            .await
            .context("read response body")?
            .to_vec())
    }

    /// GET media: attachments, embed pictures, GIF providers' files. The
    /// auth token only goes to the API host itself. The web app loads media
    /// through plain <img>/<video> tags, so Fluxer's own CDN never sees the
    /// token either, and third-party hosts such as static.klipy.com must not.
    pub async fn fetch_media_bytes(&self, url_or_path: &str) -> Result<Vec<u8>> {
        let target = self.url(url_or_path);
        if url_host(&target) == url_host(&self.base_url) {
            self.fetch_url_bytes(&target).await
        } else {
            self.fetch_public_bytes(&target).await
        }
    }

    fn url(&self, path: &str) -> String {
        if path.starts_with("http://") || path.starts_with("https://") {
            path.to_string()
        } else {
            format!("{}/{}", self.base_url, path.trim_start_matches('/'))
        }
    }
}

/// Lower-cased host of an http(s) URL, without user info or port.
fn url_host(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host_port = authority.rsplit('@').next()?;
    let host = if let Some(v6) = host_port.strip_prefix('[') {
        v6.split(']').next()?
    } else {
        host_port.split(':').next()?
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::url_host;

    #[test]
    fn url_host_compares_hosts_only() {
        assert_eq!(
            url_host("https://api.fluxer.app/v1").as_deref(),
            Some("api.fluxer.app")
        );
        assert_eq!(
            url_host("https://User:pw@API.Fluxer.app:8443/v1?x#y").as_deref(),
            Some("api.fluxer.app")
        );
        assert_eq!(url_host("http://[::1]:8080/x").as_deref(), Some("::1"));
        assert_eq!(
            url_host("https://static.klipy.com/ii/a.webp").as_deref(),
            Some("static.klipy.com")
        );
        assert_eq!(url_host("/channels/1/messages"), None);
        assert_eq!(url_host("https:///nohost"), None);
    }
}

#[cfg(test)]
mod members_failure_tests {
    use super::*;

    fn response(status: StatusCode) -> anyhow::Error {
        ApiError::Response {
            status,
            code: None,
            message: "Gateway timeout.".into(),
            body: Value::Null,
        }
        .into()
    }

    #[test]
    fn a_gateway_timeout_and_a_slow_page_are_unavailable_a_403_is_forbidden() {
        assert_eq!(
            members_failure(&response(StatusCode::GATEWAY_TIMEOUT)),
            MembersFailure::Unavailable
        );
        assert_eq!(
            members_failure(&response(StatusCode::BAD_GATEWAY)),
            MembersFailure::Unavailable
        );
        assert_eq!(
            members_failure(&MembersTimeout(45).into()),
            MembersFailure::Unavailable
        );
        assert_eq!(
            members_failure(&response(StatusCode::FORBIDDEN)),
            MembersFailure::Forbidden
        );
        assert_eq!(
            members_failure(&response(StatusCode::NOT_FOUND)),
            MembersFailure::Other
        );
        assert_eq!(
            members_failure(&anyhow!("something else")),
            MembersFailure::Other
        );
    }

    #[test]
    fn the_classification_survives_added_context() {
        let err = response(StatusCode::GATEWAY_TIMEOUT).context("members page 3");
        assert_eq!(members_failure(&err), MembersFailure::Unavailable);
    }
}
