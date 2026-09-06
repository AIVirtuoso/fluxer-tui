use crate::api::types::{
    CHANNEL_DM, CHANNEL_DM_PERSONAL_NOTES, CHANNEL_GROUP_DM, CHANNEL_GUILD_TEXT, ChannelResponse,
    MessageEmbedResponse,
};
use crate::app::{App, Focus, display_name};
use crate::ui::message_markdown;
use crate::ui::span_wrap;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use std::collections::HashMap;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

fn clip_url_for_display(url: &str, max_chars: usize) -> String {
    let t = url.trim();
    if t.is_empty() {
        return String::new();
    }
    let n = t.chars().count();
    if n <= max_chars {
        return t.to_string();
    }
    let take = max_chars.saturating_sub(1);
    format!("{}…", t.chars().take(take).collect::<String>())
}

fn embed_display_label(embed: &MessageEmbedResponse) -> (String, bool) {
    let original = embed
        .url
        .as_ref()
        .filter(|u| u.starts_with("http://") || u.starts_with("https://"));
    let t = embed.embed_type.as_str();
    let is_gif = matches!(t, "gifv")
        || original.is_some_and(|u| u.contains("tenor.com") || u.to_lowercase().ends_with(".gif"))
        || embed
            .image
            .as_ref()
            .and_then(|m| m.url.as_ref().or(m.proxy_url.as_ref()))
            .is_some_and(|u| u.to_lowercase().ends_with(".gif"));
    let label = if let Some(u) = original {
        clip_url_for_display(u, 72)
    } else {
        let proxy = embed
            .image
            .as_ref()
            .and_then(|m| m.proxy_url.clone().or_else(|| m.url.clone()))
            .or_else(|| {
                embed
                    .thumbnail
                    .as_ref()
                    .and_then(|m| m.proxy_url.clone().or_else(|| m.url.clone()))
            });
        if let Some(u) = proxy {
            clip_url_for_display(&u, 72)
        } else {
            String::new()
        }
    };
    (label, is_gif)
}

pub fn render(frame: &mut Frame, area: Rect, app: &mut App) {
    app.message_scroll_max = 0;
    if app.active_channel_is_voice() {
        render_voice(frame, area, app);
    } else if app.active_channel_is_link() {
        render_link(frame, area, app);
    } else {
        render_messages(frame, area, app);
    }
}

/// in the beginning there was GOD Just kidding it was HAMPLER.
fn channel_welcome_label(channel: &ChannelResponse) -> String {
    match channel.channel_type() {
        CHANNEL_GUILD_TEXT => format!("#{}", channel.name),
        CHANNEL_DM_PERSONAL_NOTES => "Personal Notes".to_string(),
        CHANNEL_DM => channel
            .recipients
            .first()
            .map(display_name)
            .unwrap_or_else(|| "Direct Message".to_string()),
        CHANNEL_GROUP_DM => {
            if !channel.name.trim().is_empty() {
                channel.name.clone()
            } else if !channel.recipients.is_empty() {
                channel
                    .recipients
                    .iter()
                    .map(display_name)
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                "Group DM".to_string()
            }
        }
        _ => {
            if channel.guild_id.is_some() && !channel.name.is_empty() {
                format!("#{}", channel.name)
            } else {
                channel.name.clone()
            }
        }
    }
}

/// praise satan
fn channel_welcome_lines(_app: &App, channel: &ChannelResponse) -> Vec<Line<'static>> {
    let label = channel_welcome_label(channel);
    let genesis =
        format!("In the beginning, there was nothing. Then, there was {label}. And it was good.");
    vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "Welcome to ",
                Style::default()
                    .fg(crate::ui::theme::text())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                label.clone(),
                Style::default()
                    .fg(crate::ui::theme::accent())
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(genesis, crate::ui::theme::dim_style())),
        Line::from(""),
    ]
}

fn message_was_edited(message: &crate::api::types::MessageResponse) -> bool {
    message
        .edited_timestamp
        .as_ref()
        .is_some_and(|s| !s.trim().is_empty())
}

fn edited_span() -> Span<'static> {
    Span::styled(
        "(edited) ",
        crate::ui::theme::dim_style().add_modifier(Modifier::ITALIC),
    )
}

fn sel_prefix_span(is_selected: bool) -> Span<'static> {
    if is_selected {
        Span::styled("\u{25B6} ", Style::default().fg(crate::ui::theme::accent()))
    } else {
        Span::raw("  ")
    }
}

fn truncate_to_display_width(s: &str, max_w: usize) -> String {
    if max_w == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(s) <= max_w {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0usize;
    for ch in s.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0).max(1);
        if w + cw > max_w.saturating_sub(1) {
            out.push('…');
            break;
        }
        out.push(ch);
        w += cw;
    }
    out
}

fn referenced_body_preview(ref_msg: &crate::api::types::MessageResponse) -> String {
    let c = ref_msg.content.trim();
    if !c.is_empty() {
        let flat: String = c.chars().filter(|&x| x != '\n' && x != '\r').collect();
        let count = flat.chars().count();
        let mut s: String = flat.chars().take(72).collect();
        if count > 72 {
            s.push('…');
        }
        s
    } else if !ref_msg.attachments.is_empty() {
        let n = ref_msg.attachments.len();
        if n == 1 {
            format!("[file: {}]", ref_msg.attachments[0].filename)
        } else {
            format!("[{n} attachments]")
        }
    } else if !ref_msg.embeds.is_empty() {
        "[embed]".to_string()
    } else {
        "(no text)".to_string()
    }
}

fn push_fluxer_client_system_message(
    app: &App,
    lines: &mut Vec<Line<'static>>,
    message: &crate::api::types::MessageResponse,
    is_selected_msg: bool,
) {
    let header_style = if is_selected_msg {
        Style::default().bg(crate::ui::theme::bg_tertiary())
    } else {
        Style::default()
    };
    let box_fg = crate::ui::theme::text_muted();
    let ts = format_timestamp(&message.timestamp, app.ui_settings.clock_12h);

    lines.push(
        Line::from(vec![
            sel_prefix_span(is_selected_msg),
            Span::styled("╭─ ", Style::default().fg(box_fg)),
            Span::styled(
                "Fluxerbot",
                Style::default()
                    .fg(crate::ui::theme::text())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" [SYSTEM]", Style::default().fg(crate::ui::theme::accent())),
            Span::styled(format!("  \u{2014} {ts}"), crate::ui::theme::dim_style()),
        ])
        .style(header_style),
    );

    lines.push(
        Line::from(vec![
            sel_prefix_span(is_selected_msg),
            Span::styled("┃", Style::default().fg(box_fg)),
        ])
        .style(header_style),
    );

    for row in message_markdown::content_lines(&message.content, app) {
        let mut r = vec![
            sel_prefix_span(is_selected_msg),
            Span::styled("┃ ", Style::default().fg(box_fg)),
        ];
        if row.is_empty() {
            r.push(Span::raw(" "));
        } else {
            r.extend(row);
        }
        lines.push(Line::from(r).style(header_style));
    }

    lines.push(
        Line::from(vec![
            sel_prefix_span(is_selected_msg),
            Span::styled("┃ ", Style::default().fg(box_fg)),
            Span::styled("\u{1F441}\u{FE0F} ", crate::ui::theme::dim_style()),
            Span::styled(
                "only you can see this message. ",
                crate::ui::theme::dim_style(),
            ),
            Span::styled(
                "dismiss",
                Style::default()
                    .fg(crate::ui::theme::link_color())
                    .add_modifier(Modifier::UNDERLINED),
            ),
        ])
        .style(header_style),
    );

    lines.push(
        Line::from(vec![
            sel_prefix_span(is_selected_msg),
            Span::styled("╰─", Style::default().fg(box_fg)),
        ])
        .style(header_style),
    );
}

/// Rows of one message before its left margin is added.
struct BlockRow {
    spans: Vec<Span<'static>>,
    style: Style,
    kind: RowKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RowKind {
    /// The reply context above the header.
    Context,
    Header,
    Body,
    /// Marker cells of a picture: never wrapped.
    Picture,
}

/// The left margin of a message's rows: the selection mark, then, with
/// avatars on, the avatar's four columns and a space, like the web app.
struct Margin {
    avatars: bool,
}

impl Margin {
    fn width(&self, kind: RowKind) -> usize {
        if self.avatars {
            7
        } else if matches!(kind, RowKind::Context | RowKind::Header) {
            2
        } else {
            0
        }
    }
}

const AVATAR_PLACEHOLDER: &str = "\u{2800}\u{2800}\u{2800}\u{2800}";

/// Lay out one message: wrap its rows to the text width minus the margin,
/// then put the margin in front of every row, the avatar's two rows on the
/// header row and the one after it.
fn finish_block(
    rows: Vec<BlockRow>,
    text_w: u16,
    margin: &Margin,
    is_selected: bool,
    avatar_slot: Option<usize>,
) -> Vec<Line<'static>> {
    let mut out: Vec<(Vec<Span<'static>>, Style, RowKind)> = Vec::new();
    for row in rows {
        let width = (text_w as usize)
            .saturating_sub(margin.width(row.kind))
            .max(1);
        if row.kind == RowKind::Picture {
            out.push((row.spans, row.style, row.kind));
            continue;
        }
        for spans in span_wrap::wrap_spans(&row.spans, width) {
            out.push((spans, row.style, row.kind));
        }
    }
    if out.is_empty() {
        out.push((Vec::new(), Style::default(), RowKind::Body));
    }
    let header_at = out.iter().position(|(_, _, k)| *k == RowKind::Header);
    if let (Some(_), Some(h)) = (avatar_slot, header_at)
        && h + 1 >= out.len()
    {
        // the avatar's second row needs a row under the header
        out.push((Vec::new(), Style::default(), RowKind::Body));
    }
    let muted = crate::ui::theme::muted_style();
    let mut lines = Vec::with_capacity(out.len());
    for (i, (spans, style, kind)) in out.into_iter().enumerate() {
        let mut line: Vec<Span<'static>> = Vec::with_capacity(spans.len() + 3);
        if margin.avatars {
            line.push(if i == 0 {
                sel_prefix_span(is_selected)
            } else {
                Span::raw("  ")
            });
            let avatar_row = match (avatar_slot, header_at) {
                (Some(k), Some(h)) if i == h => Some((k, 0)),
                (Some(k), Some(h)) if i == h + 1 => Some((k, 1)),
                _ => None,
            };
            match avatar_row {
                Some((k, r)) => line.push(Span::styled(
                    AVATAR_PLACEHOLDER,
                    crate::app::media_marker_style(k, r),
                )),
                None if kind == RowKind::Context && avatar_slot.is_some() => {
                    line.push(Span::styled(" \u{256D}\u{2500} ", muted));
                }
                None => line.push(Span::raw("    ")),
            }
            line.push(Span::raw(" "));
        } else if matches!(kind, RowKind::Context | RowKind::Header) {
            line.push(if i == 0 {
                sel_prefix_span(is_selected)
            } else {
                Span::raw("  ")
            });
        }
        line.extend(spans);
        lines.push(Line::from(line).style(style));
    }
    lines
}

fn context_row(
    width: usize,
    lead: &'static str,
    body: &str,
    body_style: Style,
    style: Style,
) -> BlockRow {
    let budget = width.saturating_sub(UnicodeWidthStr::width(lead)).max(1);
    BlockRow {
        spans: vec![
            Span::styled(lead, crate::ui::theme::muted_style()),
            Span::styled(truncate_to_display_width(body, budget), body_style),
        ],
        style,
        kind: RowKind::Context,
    }
}

fn header_row(
    message: &crate::api::types::MessageResponse,
    author: &str,
    name_color: ratatui::style::Color,
    style: Style,
    clock_12h: bool,
) -> BlockRow {
    let timestamp = format_timestamp(&message.timestamp, clock_12h);
    let mut spans = vec![Span::styled(
        format!("[{timestamp}] "),
        crate::ui::theme::dim_style(),
    )];
    if message_was_edited(message) {
        spans.push(edited_span());
    }
    spans.push(Span::styled(
        author.to_string(),
        Style::default().fg(name_color).add_modifier(Modifier::BOLD),
    ));
    BlockRow {
        spans,
        style,
        kind: RowKind::Header,
    }
}

fn body_row(spans: Vec<Span<'static>>) -> BlockRow {
    BlockRow {
        spans,
        style: Style::default(),
        kind: RowKind::Body,
    }
}

fn markdown_rows(rows: &mut Vec<BlockRow>, text: &str, app: &App, lead: Option<Span<'static>>) {
    for row in message_markdown::content_lines(text, app) {
        let mut spans = Vec::with_capacity(row.len() + 1);
        if let Some(lead) = &lead {
            spans.push(lead.clone());
        }
        if row.is_empty() {
            spans.push(Span::raw(" "));
        } else {
            spans.extend(row);
        }
        rows.push(body_row(spans));
    }
}

/// The marker rows of one preview; the block is registered for this draw
/// and fetched when it comes on screen.
fn push_picture_rows(
    app: &App,
    rows: &mut Vec<BlockRow>,
    pic: &crate::media::InlinePicture,
    max: (u16, u16),
    left: &mut usize,
) {
    if *left == 0 {
        return;
    }
    let (cols, prows) = crate::media::picture_cells((pic.width, pic.height), app.cell_px, max);
    let url = crate::media::proxied_url(pic, crate::media::block_px(cols, prows, app.cell_px));
    let slot = app.register_media_slot(crate::app::MediaSlot::new(
        url,
        cols,
        prows,
        crate::app::MediaKind::Picture,
    ));
    for r in 0..prows {
        rows.push(BlockRow {
            spans: vec![Span::styled(
                "\u{2800}".repeat(cols as usize),
                crate::app::media_marker_style(slot, r),
            )],
            style: Style::default(),
            kind: RowKind::Picture,
        });
    }
    *left -= 1;
}

fn build_message_lines(
    app: &App,
    messages: &[crate::api::types::MessageResponse],
    text_w: u16,
    pane_rows: u16,
) -> (Vec<Line<'static>>, Vec<(usize, usize)>) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut line_ranges = vec![(0usize, 0usize); messages.len()];
    let mut prev_author_id: Option<&str> = None;
    let mut prev_timestamp: Option<chrono::DateTime<chrono::Utc>> = None;
    let margin = Margin {
        avatars: app.avatars_enabled(),
    };
    let inline = app.inline_media_enabled();
    let body_w = (text_w as usize)
        .saturating_sub(margin.width(RowKind::Body))
        .max(1) as u16;
    let ctx_w = (text_w as usize)
        .saturating_sub(margin.width(RowKind::Context))
        .max(1);
    let picture_max = crate::media::preview_limits(body_w, pane_rows, app.cell_px);

    for (idx, message) in messages.iter().enumerate() {
        let is_selected_msg = app.selected_message_index == Some(idx);
        let cur_ts = message
            .timestamp
            .parse::<chrono::DateTime<chrono::Utc>>()
            .ok();

        if message.message_type == crate::slash_commands::MESSAGE_TYPE_CLIENT_SYSTEM
            && message.author.id == crate::slash_commands::FLUXERBOT_ID
        {
            if idx > 0 {
                lines.push(Line::from(""));
            }
            let block_start = lines.len();
            push_fluxer_client_system_message(app, &mut lines, message, is_selected_msg);
            line_ranges[idx] = (block_start, lines.len());
            prev_author_id = Some(message.author.id.as_str());
            prev_timestamp = cur_ts;
            continue;
        }

        let gid = app.guild_id_for_channel(message.channel_id.as_str());
        let author = app.shown_name_for_user(gid.as_deref(), &message.author);

        let is_self = message.author.id == app.me.id;
        let name_color = app.member_name_color(gid.as_deref(), &message.author.id, is_self);

        let has_reply_rail =
            message.referenced_message.is_some() || message.message_reference.is_some();
        let same_author = prev_author_id == Some(message.author.id.as_str());
        let within_group = same_author
            && !has_reply_rail
            && match (prev_timestamp, cur_ts) {
                (Some(prev), Some(cur)) => (cur - prev).num_minutes().abs() < 5,
                _ => false,
            };

        let header_style = if is_selected_msg {
            Style::default().bg(crate::ui::theme::bg_tertiary())
        } else {
            Style::default()
        };

        if idx > 0 && !within_group {
            lines.push(Line::from(""));
        }

        let block_start = lines.len();
        let mut rows: Vec<BlockRow> = Vec::new();
        let clock_12h = app.ui_settings.clock_12h;

        if let Some(ref_msg) = message.referenced_message.as_deref() {
            let ref_author = app.shown_name_for_user(gid.as_deref(), &ref_msg.author);
            let preview = referenced_body_preview(ref_msg);
            let ctx_body = format!("@{ref_author} - {preview}");
            rows.push(context_row(
                ctx_w,
                "\u{21AA} ",
                &ctx_body,
                crate::ui::theme::dim_style(),
                header_style,
            ));
            rows.push(header_row(
                message,
                &author,
                name_color,
                header_style,
                clock_12h,
            ));
        } else if let Some(mref) = &message.message_reference {
            let (ctx_body, body_style) = if mref.reference_type == 1 {
                (
                    "Forwarded",
                    crate::ui::theme::muted_style().add_modifier(Modifier::ITALIC),
                )
            } else {
                (
                    "(original message unavailable)",
                    crate::ui::theme::dim_style(),
                )
            };
            rows.push(context_row(
                ctx_w,
                "\u{21AA} ",
                ctx_body,
                body_style,
                header_style,
            ));
            rows.push(header_row(
                message,
                &author,
                name_color,
                header_style,
                clock_12h,
            ));
        } else if within_group && !is_selected_msg {
            // grouped under the previous message: no header
        } else if within_group && is_selected_msg {
            let timestamp = format_timestamp(&message.timestamp, clock_12h);
            let mut hdr = vec![Span::styled(
                format!("[{timestamp}] "),
                crate::ui::theme::dim_style(),
            )];
            if message_was_edited(message) {
                hdr.push(edited_span());
            }
            rows.push(BlockRow {
                spans: hdr,
                style: header_style,
                kind: RowKind::Header,
            });
        } else {
            rows.push(header_row(
                message,
                &author,
                name_color,
                header_style,
                clock_12h,
            ));
        }

        if !message.content.trim().is_empty() {
            markdown_rows(&mut rows, &message.content, app, None);
        }

        prev_author_id = Some(&message.author.id);
        prev_timestamp = cur_ts;

        let mut pictures_left = crate::media::MAX_PICTURES_PER_MESSAGE;
        for attachment in &message.attachments {
            let size_str = match attachment.size {
                Some(s) if s < 1024 => format!("{} B", s),
                Some(s) if s < 1024 * 1024 => format!("{:.1} KB", s as f64 / 1024.0),
                Some(s) => format!("{:.1} MB", s as f64 / (1024.0 * 1024.0)),
                None => "unknown".to_string(),
            };
            let mime = attachment.content_type.as_deref().unwrap_or("unknown");
            rows.push(body_row(vec![
                Span::styled(
                    "\u{1F4CE} ",
                    Style::default().fg(crate::ui::theme::accent_dim()),
                ),
                Span::styled(
                    attachment.filename.clone(),
                    Style::default()
                        .fg(crate::ui::theme::link_color())
                        .add_modifier(Modifier::UNDERLINED),
                ),
                Span::styled(
                    format!(" [{mime} \u{00B7} {size_str}]"),
                    crate::ui::theme::dim_style(),
                ),
            ]));
            if inline && let Some(pic) = crate::media::attachment_picture(attachment) {
                push_picture_rows(app, &mut rows, &pic, picture_max, &mut pictures_left);
            }
        }

        for embed in &message.embeds {
            let has_content = embed.title.is_some()
                || embed.description.is_some()
                || embed.author.is_some()
                || !embed.fields.is_empty();
            let bar_color = embed
                .color
                .map(|c| {
                    ratatui::style::Color::Rgb(
                        ((c >> 16) & 0xFF) as u8,
                        ((c >> 8) & 0xFF) as u8,
                        (c & 0xFF) as u8,
                    )
                })
                .unwrap_or(crate::ui::theme::accent_dim());
            let bar = Span::styled("\u{2502} ", Style::default().fg(bar_color));

            if !has_content {
                let (label, is_gif) = embed_display_label(embed);
                if !label.is_empty() {
                    let mut spans = vec![bar.clone()];
                    if is_gif {
                        spans.push(Span::styled(
                            "[GIF] ",
                            Style::default()
                                .fg(crate::ui::theme::accent())
                                .add_modifier(Modifier::BOLD),
                        ));
                    }
                    spans.push(Span::styled(
                        label,
                        Style::default()
                            .fg(crate::ui::theme::link_color())
                            .add_modifier(Modifier::UNDERLINED),
                    ));
                    rows.push(body_row(spans));
                }
            } else {
                if let Some(author) = &embed.author {
                    rows.push(body_row(vec![
                        bar.clone(),
                        Span::styled(author.name.clone(), crate::ui::theme::dim_style()),
                    ]));
                }
                if let Some(title) = &embed.title {
                    let mut title_spans = vec![bar.clone()];
                    let base = if embed.url.is_some() {
                        Style::default()
                            .fg(crate::ui::theme::link_color())
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                            .fg(crate::ui::theme::text())
                            .add_modifier(Modifier::BOLD)
                    };
                    for s in message_markdown::parse_message_spans(title, app) {
                        title_spans.push(Span::styled(s.content.to_string(), base.patch(s.style)));
                    }
                    rows.push(body_row(title_spans));
                }
                if let Some(desc) = &embed.description {
                    markdown_rows(&mut rows, desc, app, Some(bar.clone()));
                }
                for field in &embed.fields {
                    rows.push(body_row(vec![
                        bar.clone(),
                        Span::styled(
                            field.name.clone(),
                            Style::default()
                                .fg(crate::ui::theme::text())
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]));
                    markdown_rows(&mut rows, &field.value, app, Some(bar.clone()));
                }
                if let Some(footer) = &embed.footer {
                    rows.push(body_row(vec![
                        bar.clone(),
                        Span::styled(footer.text.clone(), crate::ui::theme::muted_style()),
                    ]));
                }
            }
            if inline && let Some(pic) = crate::media::embed_picture(embed) {
                push_picture_rows(app, &mut rows, &pic, picture_max, &mut pictures_left);
            }
        }

        if !message.reactions.is_empty() {
            let mut reaction_spans: Vec<Span<'static>> = Vec::new();
            for reaction in &message.reactions {
                let style = if reaction.me {
                    Style::default()
                        .fg(crate::ui::theme::accent())
                        .add_modifier(Modifier::BOLD)
                } else {
                    crate::ui::theme::dim_style()
                };
                // custom emoji: the picture where the terminal can draw one
                let picture = reaction
                    .emoji
                    .id
                    .as_deref()
                    .and_then(|id| app.custom_emoji_placeholder(id, reaction.emoji.animated));
                match picture {
                    Some(p) => {
                        reaction_spans.push(Span::styled(" ", style));
                        reaction_spans.push(p);
                        reaction_spans.push(Span::styled(format!(" {}", reaction.count), style));
                    }
                    None => {
                        let emoji_str = if reaction.emoji.id.is_some() {
                            format!(":{}:", reaction.emoji.name)
                        } else {
                            reaction.emoji.name.clone()
                        };
                        reaction_spans.push(Span::styled(
                            format!(" {emoji_str} {}", reaction.count),
                            style,
                        ));
                    }
                }
                reaction_spans.push(Span::raw(" "));
            }
            rows.push(body_row(reaction_spans));
        }

        let avatar_slot = if margin.avatars && !within_group {
            Some(app.register_media_slot(app.avatar_slot(
                gid.as_deref(),
                &message.author,
                message.member.as_ref().and_then(|m| m.avatar.as_deref()),
            )))
        } else {
            None
        };
        lines.extend(finish_block(
            rows,
            text_w,
            &margin,
            is_selected_msg,
            avatar_slot,
        ));

        line_ranges[idx] = (block_start, lines.len());
    }

    (lines, line_ranges)
}

fn paragraph_line_heights(lines: &[Line<'static>], text_w: u16) -> Vec<u16> {
    lines
        .iter()
        .map(|line| {
            Paragraph::new(Text::from(vec![line.clone()]))
                .wrap(Wrap { trim: false })
                .line_count(text_w) as u16
        })
        .collect()
}

/// Everything that decides what the message pane's lines look like. While
/// it stays the same from one draw to the next, the lines are reused.
#[derive(Clone, PartialEq, Eq)]
pub struct LayoutKey {
    channel: Option<String>,
    text_w: u16,
    pane_rows: u16,
    selected: Option<usize>,
    /// Hash of the messages: content, edits, reactions, embeds, members.
    messages: u64,
    clock_12h: bool,
    avatars: bool,
    inline: bool,
    theme: crate::config::Theme,
    cell_px: (u32, u32),
    roster: u64,
    emoji: u64,
}

/// The message pane's lines as built for one [`LayoutKey`], with what the
/// build registered on the side: the media and emoji slots its marker
/// cells refer to, and the emoji it wanted fetched.
pub struct PaneLayout {
    key: LayoutKey,
    pub lines: Vec<Line<'static>>,
    /// Rows before each line, and after the last: `cum[i]` is where line `i`
    /// starts, `cum[lines.len()]` the body's height.
    pub cum: Vec<u32>,
    pub line_ranges: Vec<(usize, usize)>,
    media_slots: Vec<crate::app::MediaSlot>,
    emoji_slots: Vec<String>,
    emoji_wants: Vec<(String, bool)>,
}

impl std::fmt::Debug for PaneLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PaneLayout({} lines)", self.lines.len())
    }
}

/// The pane's lines for these messages, built now or reused from the last
/// draw. Building takes tens of milliseconds for a few hundred messages,
/// which would cap scrolling; a plain scroll changes nothing in the key.
pub fn pane_layout(
    app: &App,
    messages: &[crate::api::types::MessageResponse],
    text_w: u16,
    pane_rows: u16,
) -> std::rc::Rc<PaneLayout> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    messages.hash(&mut hasher);
    let key = LayoutKey {
        channel: app.selected_channel_id.clone(),
        text_w,
        pane_rows,
        selected: app.selected_message_index,
        messages: hasher.finish(),
        clock_12h: app.ui_settings.clock_12h,
        avatars: app.avatars_enabled(),
        inline: app.inline_media_enabled(),
        theme: app.ui_settings.theme,
        cell_px: app.cell_px,
        roster: app.roster_version,
        emoji: app.custom_emoji_version,
    };
    if let Some(layout) = app.pane_layout.borrow().as_ref()
        && layout.key == key
    {
        // the marker cells in the lines count on these slot tables
        *app.media_slots.borrow_mut() = layout.media_slots.clone();
        *app.custom_emoji_slots.borrow_mut() = layout.emoji_slots.clone();
        let mut wanted = app.custom_emoji_wanted.borrow_mut();
        for want in &layout.emoji_wants {
            if !wanted.iter().any(|w| w.0 == want.0) {
                wanted.push(want.clone());
            }
        }
        return layout.clone();
    }
    app.media_slots.borrow_mut().clear();
    app.custom_emoji_slots.borrow_mut().clear();
    let (lines, line_ranges) = build_message_lines(app, messages, text_w, pane_rows);
    let heights = paragraph_line_heights(&lines, text_w);
    let mut cum = Vec::with_capacity(lines.len() + 1);
    cum.push(0u32);
    for h in heights {
        cum.push(cum.last().copied().unwrap_or(0) + h as u32);
    }
    let layout = std::rc::Rc::new(PaneLayout {
        key,
        lines,
        cum,
        line_ranges,
        media_slots: app.media_slots.borrow().clone(),
        emoji_slots: app.custom_emoji_slots.borrow().clone(),
        emoji_wants: app.custom_emoji_wanted.borrow().clone(),
    });
    *app.pane_layout.borrow_mut() = Some(layout.clone());
    layout
}

/// The rows of the pane from top to bottom: blank filler when the content
/// is shorter than the pane, the channel welcome, a gap, then the body.
struct RowModel<'a> {
    filler: u32,
    welcome: &'a [Line<'static>],
    welcome_heights: &'a [u16],
    gap: u32,
    body: &'a [Line<'static>],
    body_cum: &'a [u32],
}

impl RowModel<'_> {
    fn welcome_rows(&self) -> u32 {
        self.welcome_heights.iter().map(|&h| h as u32).sum()
    }

    /// Rows before the body.
    fn pre_rows(&self) -> u32 {
        self.filler + self.welcome_rows() + self.gap
    }

    fn body_rows(&self) -> u32 {
        self.body_cum.last().copied().unwrap_or(0)
    }

    fn total(&self) -> u32 {
        self.pre_rows() + self.body_rows()
    }

    /// The lines that cover rows `top..top + rows`, and how many rows of the
    /// first one lie above `top`.
    fn window(&self, top: u32, rows: u16) -> (Vec<Line<'static>>, u16) {
        let need = top + rows as u32;
        let mut out: Vec<Line<'static>> = Vec::new();
        let mut offset = 0u32;
        let mut row = 0u32;
        let pre: Vec<(Line<'static>, u32)> = (0..self.filler)
            .map(|_| (Line::from(""), 1))
            .chain(
                self.welcome
                    .iter()
                    .zip(self.welcome_heights)
                    .map(|(l, &h)| (l.clone(), h as u32)),
            )
            .chain((0..self.gap).map(|_| (Line::from(""), 1)))
            .collect();
        for (line, h) in pre {
            let end = row + h;
            if end > top {
                if out.is_empty() {
                    offset = top.saturating_sub(row);
                }
                out.push(line);
            }
            row = end;
            if row >= need {
                return (out, offset as u16);
            }
        }
        let pre_rows = row;
        let first = if top > pre_rows {
            self.body_cum
                .partition_point(|&c| pre_rows + c <= top)
                .saturating_sub(1)
        } else {
            0
        };
        for i in first..self.body.len() {
            let start = pre_rows + self.body_cum[i];
            let end = pre_rows + self.body_cum[i + 1];
            if end <= top {
                continue;
            }
            if out.is_empty() {
                offset = top.saturating_sub(start);
            }
            out.push(self.body[i].clone());
            if end >= need {
                break;
            }
        }
        (out, offset as u16)
    }
}

#[cfg(test)]
mod row_model_tests {
    use super::*;

    fn texts(lines: &[Line<'static>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn window_walks_filler_welcome_gap_and_body_by_rows() {
        let welcome = [Line::from("w0"), Line::from("w1")];
        let body = [
            Line::from("b0"),
            Line::from("b1 (two rows)"),
            Line::from("b2"),
        ];
        let model = RowModel {
            filler: 2,
            welcome: &welcome,
            welcome_heights: &[1, 1],
            gap: 1,
            body: &body,
            body_cum: &[0, 1, 3, 4],
        };
        assert_eq!(model.pre_rows(), 5);
        assert_eq!(model.total(), 9);
        let (lines, offset) = model.window(0, 4);
        assert_eq!(texts(&lines), ["", "", "w0", "w1"]);
        assert_eq!(offset, 0);
        let (lines, offset) = model.window(4, 3);
        assert_eq!(texts(&lines), ["", "b0", "b1 (two rows)"]);
        assert_eq!(offset, 0);
        let (lines, offset) = model.window(6, 2);
        assert_eq!(texts(&lines), ["b1 (two rows)"]);
        assert_eq!(offset, 0);
        let (lines, offset) = model.window(7, 2);
        assert_eq!(
            texts(&lines),
            ["b1 (two rows)", "b2"],
            "starts inside the tall line"
        );
        assert_eq!(offset, 1);
        let (lines, _) = model.window(8, 5);
        assert_eq!(texts(&lines), ["b2"]);
    }
}

/// The message whose block holds row `top` (else the first one below it),
/// and how far into it `top` lies: where the reader is.
fn anchor_at(
    top: u32,
    pre_rows: u32,
    line_ranges: &[(usize, usize)],
    cum: &[u32],
    ids: &[&str],
) -> Option<(String, i64)> {
    for (i, &(b0, b1)) in line_ranges.iter().enumerate() {
        let (Some(&start), Some(&end)) = (cum.get(b0), cum.get(b1)) else {
            continue;
        };
        let (start, end) = (pre_rows + start, pre_rows + end);
        if top < end || i + 1 == line_ranges.len() {
            let id = ids.get(i)?;
            return Some((id.to_string(), top as i64 - start as i64));
        }
    }
    None
}

/// The top row that puts the anchored message back where it was, in a
/// layout that may have changed around it.
fn top_for_anchor(
    message_id: &str,
    offset: i64,
    pre_rows: u32,
    line_ranges: &[(usize, usize)],
    cum: &[u32],
    ids: &[&str],
    max_scroll: u32,
) -> Option<u32> {
    let i = ids.iter().position(|id| *id == message_id)?;
    let &(b0, _) = line_ranges.get(i)?;
    let start = pre_rows as i64 + *cum.get(b0)? as i64;
    Some((start + offset).clamp(0, max_scroll as i64) as u32)
}

#[cfg(test)]
mod anchor_tests {
    use super::*;

    // three messages of 2, 3 and 1 rows after 4 rows of welcome
    const RANGES: [(usize, usize); 3] = [(0, 2), (2, 5), (5, 6)];
    const CUM: [u32; 7] = [0, 1, 2, 3, 4, 5, 6];
    const IDS: [&str; 3] = ["a", "b", "c"];

    #[test]
    fn the_anchor_is_the_message_under_the_top_row() {
        assert_eq!(anchor_at(0, 4, &RANGES, &CUM, &IDS), Some(("a".into(), -4)));
        assert_eq!(anchor_at(5, 4, &RANGES, &CUM, &IDS), Some(("a".into(), 1)));
        assert_eq!(anchor_at(6, 4, &RANGES, &CUM, &IDS), Some(("b".into(), 0)));
        assert_eq!(anchor_at(8, 4, &RANGES, &CUM, &IDS), Some(("b".into(), 2)));
        assert_eq!(anchor_at(9, 4, &RANGES, &CUM, &IDS), Some(("c".into(), 0)));
        assert_eq!(
            anchor_at(30, 4, &RANGES, &CUM, &IDS),
            Some(("c".into(), 21))
        );
    }

    #[test]
    fn a_new_message_below_does_not_move_the_reader() {
        // the reader is 1 row into "a"; a 4-row message "d" arrives below
        let (id, offset) = anchor_at(5, 4, &RANGES, &CUM, &IDS).unwrap();
        let ranges = [(0, 2), (2, 5), (5, 6), (6, 10)];
        let cum = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let ids = ["a", "b", "c", "d"];
        assert_eq!(
            top_for_anchor(&id, offset, 4, &ranges, &cum, &ids, 100),
            Some(5)
        );
        // older messages loaded above push the anchor down the pane by their rows
        let ranges = [(0, 3), (3, 5), (5, 8), (8, 9)];
        let cum = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let ids = ["old", "a", "b", "c"];
        assert_eq!(
            top_for_anchor(&id, offset, 4, &ranges, &cum, &ids, 100),
            Some(8)
        );
        // never past the end of the content
        assert_eq!(
            top_for_anchor(&id, offset, 4, &ranges, &cum, &ids, 6),
            Some(6)
        );
        assert_eq!(top_for_anchor("gone", 0, 4, &ranges, &cum, &ids, 6), None);
    }
}

/// When a message is selected, adjust `message_scroll_from_bottom` so the selection stays in view.
pub fn scroll_for_selected_message(
    app: &App,
    text_w: u16,
    pane_visible: u16,
    current_scroll_from_bottom: u16,
) -> Option<u16> {
    let idx = app.selected_message_index?;
    let messages = app.active_messages();
    if messages.is_empty() || idx >= messages.len() {
        return None;
    }
    if app
        .selected_channel_id
        .as_ref()
        .is_some_and(|id| app.loading_messages.contains(id))
    {
        return None;
    }

    let welcome: Vec<Line<'static>> = app
        .active_channel()
        .map(|ch| channel_welcome_lines(app, &ch))
        .unwrap_or_default();
    let welcome_heights = paragraph_line_heights(&welcome, text_w);
    let layout = pane_layout(app, &messages, text_w, pane_visible);
    let mut model = RowModel {
        filler: 0,
        welcome: &welcome,
        welcome_heights: &welcome_heights,
        gap: u32::from(!welcome.is_empty()),
        body: &layout.lines,
        body_cum: &layout.cum,
    };
    let pane = pane_visible as u32;
    model.filler = pane.saturating_sub(model.total());
    let total = model.total();
    if total <= pane {
        return Some(0);
    }

    let max_scroll = total - pane;
    let scroll = (current_scroll_from_bottom as u32).min(max_scroll);
    let mut top = max_scroll - scroll;

    let (b0, b1) = layout.line_ranges.get(idx).copied().unwrap_or((0, 0));
    if b1 >= layout.cum.len() {
        return None;
    }
    let pre = model.pre_rows();
    let rs = pre + layout.cum[b0];
    let re = pre + layout.cum[b1];

    if re.saturating_sub(rs) > pane {
        top = rs;
    } else {
        if rs < top {
            top = rs;
        }
        if re > top + pane {
            top = re - pane;
        }
    }

    top = top.min(max_scroll);
    let new_scroll = max_scroll - top;
    Some(new_scroll.min(u16::MAX as u32) as u16)
}

fn render_messages(frame: &mut Frame, area: Rect, app: &mut App) {
    let block = block("Messages", app.focus == Focus::Messages);
    let inner = block.inner(area);
    let text_w = inner.width.max(1);

    let pane_visible = area.height.saturating_sub(2).max(1);

    let welcome: Vec<Line<'static>> = app
        .active_channel()
        .map(|ch| channel_welcome_lines(app, &ch))
        .unwrap_or_default();
    let welcome_heights = paragraph_line_heights(&welcome, text_w);

    let messages = app.active_messages();
    let loading = app
        .selected_channel_id
        .as_ref()
        .is_some_and(|id| app.loading_messages.contains(id));

    let layout = if loading || messages.is_empty() {
        None
    } else {
        Some(pane_layout(app, &messages, text_w, pane_visible))
    };
    let loading_line = [Line::from(Span::styled(
        "Loading messages...",
        crate::ui::theme::dim_style(),
    ))];
    let (body, body_cum): (&[Line<'static>], &[u32]) = match &layout {
        Some(l) => (&l.lines, &l.cum),
        None if loading => (&loading_line, &[0, 1]),
        None => (&[], &[0]),
    };
    let mut model = RowModel {
        filler: 0,
        welcome: &welcome,
        welcome_heights: &welcome_heights,
        gap: u32::from(!welcome.is_empty()),
        body,
        body_cum,
    };
    let pane = pane_visible as u32;
    model.filler = pane.saturating_sub(model.total());
    let total = model.total();

    let max_scroll = total.saturating_sub(pane);
    app.message_scroll_max = max_scroll.min(u16::MAX as u32) as u16;
    let mut scroll_from_bottom = (app.message_scroll_from_bottom as u32).min(max_scroll);
    let ids: Vec<&str> = messages.iter().map(|m| m.id.as_str()).collect();
    let pre_rows = model.pre_rows();
    // Scrolled up, and the content changed since the last draw (a message
    // arrived, history loaded, an edit): put the reader back on the same
    // message rather than let the bottom drag the view.
    if scroll_from_bottom > 0
        && let (Some(anchor), Some(layout)) = (&app.pane_anchor, &layout)
        && anchor.channel == app.selected_channel_id
        && anchor.total_rows != total
        && let Some(new_top) = top_for_anchor(
            &anchor.message_id,
            anchor.offset,
            pre_rows,
            &layout.line_ranges,
            &layout.cum,
            &ids,
            max_scroll,
        )
    {
        scroll_from_bottom = max_scroll - new_top;
        app.message_scroll_from_bottom = scroll_from_bottom.min(u16::MAX as u32) as u16;
    }
    let top = max_scroll - scroll_from_bottom;
    app.pane_anchor = layout.as_ref().and_then(|layout| {
        let (message_id, offset) =
            anchor_at(top, pre_rows, &layout.line_ranges, &layout.cum, &ids)?;
        Some(crate::app::PaneAnchor {
            channel: app.selected_channel_id.clone(),
            message_id,
            offset,
            total_rows: total,
        })
    });

    // The same content in the same place, just scrolled: the terminal can
    // shift the rows itself and keep the pictures in them.
    let view = crate::app::PaneView {
        channel: app.selected_channel_id.clone(),
        inner,
        total_rows: total.min(u16::MAX as u32) as u16,
        top: top.min(u16::MAX as u32) as u16,
    };
    app.pane_scroll_hint = match &app.pane_last {
        Some(last)
            if last.channel == view.channel
                && last.inner == view.inner
                && last.total_rows == view.total_rows
                && last.top != view.top =>
        {
            Some(crate::console::backend::RegionScroll {
                area: inner,
                rows: view.top as i32 - last.top as i32,
            })
        }
        _ => None,
    };
    app.pane_last = Some(view);

    // only the lines on screen are handed to the paragraph
    let (lines, offset) = model.window(top, pane_visible);
    let paragraph = Paragraph::new(Text::from(lines))
        .block(block.clone())
        .wrap(Wrap { trim: false })
        .scroll((offset, 0));
    frame.render_widget(paragraph, area);
}

/// Draw custom emoji pictures over the marked placeholder cells the paragraph
/// just laid out. Scanning the buffer means wrapping and scrolling are already
/// accounted for.
pub fn overlay_custom_emojis(frame: &mut Frame, inner: Rect, app: &App) {
    let slots = app.custom_emoji_slots.borrow();
    if slots.is_empty() {
        return;
    }
    let w = crate::app::CUSTOM_EMOJI_CELLS;
    let buf = frame.buffer_mut();
    for y in inner.y..inner.y.saturating_add(inner.height) {
        let mut x = inner.x;
        while x.saturating_add(w) <= inner.x.saturating_add(inner.width) {
            let slot = crate::app::custom_emoji_marker_slot(buf[(x, y)].style());
            let Some(k) = slot else {
                x += 1;
                continue;
            };
            // both cells must belong to the same placeholder (a wrap could
            // split one; then draw nothing rather than over a neighbour)
            let whole = (1..w).all(|dx| {
                crate::app::custom_emoji_marker_slot(buf[(x + dx, y)].style()) == Some(k)
            });
            if whole
                && let Some(id) = slots.get(k)
                && let Some((serial, frame_idx, picture)) = app.custom_emoji_current(id)
            {
                let rect = Rect::new(x, y, w, 1);
                match picture {
                    crate::app::Picture::Terminal(tp) => {
                        place_terminal_picture(app, buf, rect, serial, frame_idx, tp);
                    }
                    crate::app::Picture::Pixels(img) => {
                        app.pixel_placements
                            .borrow_mut()
                            .push(crate::console::raster::Placement {
                                area: rect,
                                image: img.clone(),
                            });
                    }
                }
            }
            x += w;
        }
    }
}

/// Put a terminal picture on a block of cells. The cells are skipped, so
/// the text under them is never rewritten while the picture is there; the
/// rows that carry escape sequences get a sentinel cell, and the backend
/// prints the picture at it instead of the cell.
fn place_terminal_picture(
    app: &App,
    buf: &mut ratatui::buffer::Buffer,
    rect: Rect,
    serial: u16,
    frame: usize,
    picture: &crate::app::TerminalPicture,
) {
    if picture.area.width > rect.width || picture.area.height > rect.height {
        // encoded for a bigger block than it has: printing it would spill
        return;
    }
    let bottom = rect.y.saturating_add(rect.height);
    for y in rect.y..bottom {
        for x in rect.x..rect.x.saturating_add(rect.width) {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_skip(true);
            }
        }
    }
    let mut pictures = app.terminal_pictures.borrow_mut();
    for (dy, data) in &picture.rows {
        let y = rect.y.saturating_add(*dy);
        if y >= bottom {
            continue;
        }
        if let Some(cell) = buf.cell_mut((rect.x, y)) {
            let style = crate::app::picture_sentinel_style(cell.style(), serial, frame);
            cell.set_skip(false);
            cell.set_style(style);
        }
        pictures.insert(
            (rect.x, y),
            crate::console::backend::PicturePrint {
                data: data.clone(),
                area: rect,
            },
        );
    }
}

/// Draw the pictures whose marker blocks the message pane laid out this
/// draw, where the whole block is on screen (a block cut by the pane's edge
/// draws nothing until it scrolls fully in), and ask for the ones that are
/// not loaded yet. Scanning the buffer means wrapping and scrolling are
/// already accounted for.
pub fn overlay_media(frame: &mut Frame, area: Rect, app: &App) {
    let slots = app.media_slots.borrow();
    if slots.is_empty() {
        return;
    }
    struct Seen {
        x: u16,
        top: u16,
        rows: u32,
        consistent: bool,
    }
    let draw = app.draw_serial.get();
    let buf = frame.buffer_mut();
    let mut seen: HashMap<usize, Seen> = HashMap::new();
    for y in area.y..area.y.saturating_add(area.height) {
        let right = area.x.saturating_add(area.width);
        let mut x = area.x;
        while x < right {
            let Some((k, r)) = crate::app::media_marker(buf[(x, y)].style()) else {
                x += 1;
                continue;
            };
            let mut run: u16 = 1;
            while x.saturating_add(run) < right
                && crate::app::media_marker(buf[(x + run, y)].style()) == Some((k, r))
            {
                run += 1;
            }
            if let Some(slot) = slots.get(k)
                && run == slot.cols
                && y >= r
            {
                let top = y - r;
                let entry = seen.entry(k).or_insert(Seen {
                    x,
                    top,
                    rows: 0,
                    consistent: true,
                });
                if entry.x != x || entry.top != top {
                    entry.consistent = false;
                }
                entry.rows |= 1 << r.min(31);
            }
            x = x.saturating_add(run);
        }
    }
    let mut wanted = app.media_wanted.borrow_mut();
    for (k, block) in seen {
        let slot = &slots[k];
        let whole = block.consistent && block.rows == (1u32 << slot.rows.min(31)) - 1;
        match app.media.lookup(&slot.key, draw) {
            crate::media::Lookup::Ready(frames) => {
                if !whole {
                    continue;
                }
                let animated = !app.ui_settings.performance_mode && frames.is_animated();
                let (frame_idx, picture) = if animated {
                    app.note_animation(frames);
                    frames.current(std::time::Instant::now(), draw)
                } else {
                    (0, &frames.frames[0])
                };
                let rect = Rect::new(block.x, block.top, slot.cols, slot.rows);
                match picture {
                    crate::app::Picture::Terminal(tp) => {
                        place_terminal_picture(app, buf, rect, frames.serial, frame_idx, tp);
                    }
                    crate::app::Picture::Pixels(img) => {
                        app.pixel_placements
                            .borrow_mut()
                            .push(crate::console::raster::Placement {
                                area: rect,
                                image: img.clone(),
                            });
                    }
                }
                if animated {
                    app.media_animation_seen.set(true);
                }
            }
            crate::media::Lookup::Missing => {
                if !wanted.iter().any(|w| w.key == slot.key) {
                    wanted.push(slot.clone());
                }
            }
            crate::media::Lookup::Pending => {}
        }
    }
}

fn render_voice(frame: &mut Frame, area: Rect, app: &App) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(channel) = app.active_channel() {
        lines.push(Line::styled(
            format!("\u{1F50A} {}", channel.name),
            Style::default()
                .fg(crate::ui::theme::voice_color())
                .add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::from(""));
        lines.push(Line::styled(
            "Voice is view-only in this client: you cannot join, transmit, or hear audio here.",
            crate::ui::theme::dim_style(),
        ));
        lines.push(Line::styled(
            "Below is who appears connected from gateway state (informational only).",
            crate::ui::theme::muted_style(),
        ));
        lines.push(Line::from(""));
        lines.push(Line::styled(
            "Members",
            Style::default()
                .fg(crate::ui::theme::accent())
                .add_modifier(Modifier::BOLD),
        ));

        let members = app.voice_members_for_active_channel();
        if members.is_empty() {
            lines.push(Line::styled(
                "Nobody listed.",
                crate::ui::theme::dim_style(),
            ));
        } else {
            for member in members {
                lines.push(Line::styled(
                    format!("  {member}"),
                    Style::default().fg(crate::ui::theme::text()),
                ));
            }
        }
    } else {
        lines.push(Line::styled(
            "Select a voice channel.",
            crate::ui::theme::dim_style(),
        ));
    }

    let paragraph = Paragraph::new(Text::from(lines))
        .block(block("Voice (read-only)", app.focus == Focus::Messages));
    frame.render_widget(paragraph, area);
}

fn render_link(frame: &mut Frame, area: Rect, app: &App) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(channel) = app.active_channel() {
        lines.push(Line::styled(
            format!("\u{1F517} {}", channel.name),
            Style::default()
                .fg(crate::ui::theme::link_color())
                .add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::from(""));
        if let Some(url) = &channel.url {
            lines.push(Line::styled(
                url.clone(),
                Style::default().fg(crate::ui::theme::accent()),
            ));
            lines.push(Line::from(""));
            lines.push(Line::styled(
                "Press Enter to open this link in your browser.",
                crate::ui::theme::dim_style(),
            ));
        } else {
            lines.push(Line::styled(
                "This link channel has no URL set.",
                crate::ui::theme::dim_style(),
            ));
        }
    } else {
        lines.push(Line::styled(
            "Select a channel.",
            crate::ui::theme::dim_style(),
        ));
    }

    let paragraph =
        Paragraph::new(Text::from(lines)).block(block("Link", app.focus == Focus::Messages));
    frame.render_widget(paragraph, area);
}

fn format_timestamp(raw: &str, clock_12h: bool) -> String {
    use chrono::{DateTime, Local, Utc};

    if let Ok(dt) = raw.parse::<DateTime<Utc>>() {
        let local = dt.with_timezone(&Local);
        let now = Local::now();
        if clock_12h {
            if local.date_naive() == now.date_naive() {
                let h = local.format("%I").to_string();
                let h = h.trim_start_matches('0');
                let h = if h.is_empty() { "12" } else { h };
                return format!("{h}{}", local.format(":%M %p"));
            }
            let h = local.format("%I").to_string();
            let h = h.trim_start_matches('0');
            let h = if h.is_empty() { "12" } else { h };
            let tail = local.format(":%M %p").to_string();
            return format!("{}{h}{tail}", local.format("%m/%d "));
        }
        if local.date_naive() == now.date_naive() {
            return local.format("%H:%M").to_string();
        }
        return local.format("%m/%d %H:%M").to_string();
    }

    if raw.len() >= 16 {
        raw[11..16].to_string()
    } else {
        raw.to_string()
    }
}

fn block(title: &str, focused: bool) -> Block<'static> {
    Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .border_style(crate::ui::theme::focused_border(focused))
        .style(Style::default().bg(crate::ui::theme::bg()))
}
