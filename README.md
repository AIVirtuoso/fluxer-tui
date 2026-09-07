# Fluxer TUI

TUI for [Fluxer](https://fluxer.app), built with Ratatui.

## What this fork adds

This is a fork of [dogbonewish/fluxer-tui](https://github.com/dogbonewish/fluxer-tui).
It keeps upstream's config file, keys and slash commands and adds the
following on top of upstream's master (April 2026). Each item has its
own section or table row further down.

**Login and packaging**

- **Browser login works again.** The handoff poll sends the
  `poll_secret` that the Fluxer API introduced in September 2026;
  without it the poll stays "pending" forever and trips the per-IP
  lockout within seconds.
- **A Nix flake** with the package, a dev shell and a `dev` app that
  tries a branch without recompiling every dependency; `cargo install
  --git` names the package (see "Install with cargo" and "Trying a
  branch without a full rebuild").

**Look**

- **The terminal's own colours by default.** The client uses the
  terminal's foreground, background and 16 ANSI colours, so it follows
  the terminal's theme, light or dark. `[ui] theme = "fluxer"`, or the
  Colours row in the settings overlay (**F2**), keeps upstream's fixed
  dark look.
- **Profile pictures beside messages, and pictures, GIFs and video
  posters under them**, like the web app, on sixel, kitty and iTerm2
  terminals and on the console. Previews are scaled by the media proxy
  before download and cached in memory and on disk within a budget; a
  picture cut by the pane's edge shows the part that is on screen (see
  "Pictures in chat"). Upstream shows a picture only on **Ctrl+O**.
- **Emoji shortcodes the Fluxer way.** About 900 of Fluxer's emoji
  names (`:slight_smile:`, `:thumbsup:`, ...) are unknown to the
  `emojis` crate and used to stay as raw text; they now render, complete
  in place while typing, and go out as the emoji. The `:` popup lists
  an exact name first, then prefixes, then substrings.
- **Custom community emoji as pictures.** `<:name:id>` in messages,
  reactions, the compose box and the `:` popup draws the actual emoji
  in terminals with graphics; animated ones play, every instance in
  step. Terminals without graphics keep the `:name:` text.
- **GIFs from the picker (KLIPY, Tenor) play in the terminal.** Upstream
  handed the provider's web page to an external player, so nothing
  played. Animated WebP is decoded as well as GIF, and the auth token
  is sent to the API host only, never to third-party media hosts.

**Reading**

- **The view stays still while you read history.** New messages and
  loaded history no longer push what you are reading up the pane; **G**
  jumps to the newest message.
- **Profile view.** **p** on a selected message shows its author's
  profile: badges, pronouns, bio, when they joined Fluxer and the
  community, roles, connections, mutual communities and friends. **p**
  again shows the profile picture full size.
- **Cheap scrolling.** The terminal shifts rows itself, only the rows
  that changed are sent, each frame goes out as one synchronized
  update, and a held key scrolls as fast as the terminal keeps up.
- **Messages drawn with Fluxer's own markup rules:** fenced code with a
  language, lists, tables, the five callouts, quotes, headings, small
  print, every inline mark in both spellings, backslash escapes, bare
  links, masked links that show where they go, timestamps in all nine
  styles (see "Message formatting"). Upstream's renderer knew a handful
  of inline marks and drew the rest as typed.
- **Mentions and replies highlighted** the way the web app does it: an
  amber bar in the margin (and a tint in the Fluxer theme) on messages
  that mention you or answer you, and the message a selected reply
  answers is marked (see "Interface overview").
- **A performance mode that means it:** no pictures, no animations,
  slower ticks and one frame per burst of gateway traffic, for slow
  machines (see "Tips"). Upstream's skipped little.
- **Fixes:** the newest message is no longer cut off at the bottom of
  the pane; a sixel profile picture cut by the pane's edge leaves no
  stale strip behind; a community's notification settings save again
  (the API's answer to a change could not be read, so every change
  looked failed, and a saved entry emptied the client at the next
  start); and a community whose member list times out or is off limits
  no longer trips the client up, which keeps what arrived and says so.

**Sending**

- **Attach any kind of file.** A file picker (**Ctrl+F** or `/attach`),
  `/attach <path>`, and **Ctrl+V** for the image or the files on the
  clipboard; up to ten per message, with previews of staged pictures
  and videos (see "Attaching files"). Upstream sends text only.

**Sound and notifications**

- **Audio attachments play** with **Ctrl+O** through an external player
  (mpv, ffplay, pw-play, paplay, aplay, or one you name), the same way
  in a terminal emulator and on the console (see "Audio").
- **Notifications** for mentions and direct messages through libnotify's
  `notify-send`, or GNU `mail` for the console when you opt in (see
  "Notifications").

**Console**

- **Console mode.** On a Linux virtual console (`TERM=linux`) the whole
  UI is drawn through DRM/KMS with real fonts, colour emoji, custom and
  animated emoji, pictures and GIFs; no terminal emulator is needed
  (see "Console mode").

## Requirements

- Rust toolchain
- A terminal with reasonable size (the layout expects multiple panes)
- Network access for the API, gateway WebSocket, and browser login

## Build and run

```bash
cargo build --release
# binary: target/release/fluxer-tui
cargo run --release
```

## Install with cargo

The fork installs straight from git, like upstream. Name the package: the
repository also holds `scripts/gen-emoji-aliases`, a small tool that
generates the emoji alias table, and cargo asks which one to install
otherwise.

```bash
cargo install --git https://github.com/AIVirtuoso/fluxer-tui fluxer-tui
# a branch: cargo install --git https://github.com/AIVirtuoso/fluxer-tui --branch <branch> fluxer-tui
```

This puts `fluxer-tui` in `~/.cargo/bin`. The crate uses edition 2024, so
Rust 1.85 or newer is needed; the console-mode dependencies (drm, swash)
are pure Rust, so no C libraries have to be installed. A few tools are
looked up on PATH at runtime and are optional: `chafa` for text-art
pictures on a terminal without a graphics protocol, `wl-copy` or `xclip`
for pasting, and `fc-match` (fontconfig) only when running on a Linux
virtual console, where the UI is drawn through DRM (see `[console]`
below).

## Trying a branch without a full rebuild

`nix run github:AIVirtuoso/fluxer-tui/<branch>` builds the package in the
Nix sandbox, where every new commit compiles all dependencies again. For
quick tests use the `dev` app instead:

```sh
nix run --refresh github:AIVirtuoso/fluxer-tui/<branch>#dev
```

It copies the snapshot to `~/.cache/fluxer-tui/src-…` (cargo tells fresh
from stale by file dates, and store files have none) and builds it with
cargo in `~/.cache/fluxer-tui/target`, so only the crates that changed are
recompiled (the first run compiles everything once, in release mode). `--refresh` makes Nix look the branch up again instead of
reusing the commit it cached for an hour. From a checkout, `nix run .#dev`
does the same.

## Community
   
   > *fluxer-tui running on a vintage Apple iBook - courtesy of @astromahdi#2602*
   
   ![fluxer-tui on iBook](assets/IMG_7484.jpeg)

## First-time login

If you have no valid saved token then you can easily login via the browser. The TUI will automatically:

1. Print an **8-character login code** (also copied to the clipboard when possible).
2. Opens your browser to complete login (or you can open the printed URL manually).
3. Polls until the browser flow finishes, then saves the token and starts the UI.

You can pass a token once without storing it in config:

```bash
target/release/fluxer-tui --token 'YOUR_TOKEN_HERE'
```

To clear the saved token and exit:

```bash
target/release/fluxer-tui/fluxer-tui --logout
```

Or you can CTRL + L (see below)

## Command-line options

| Option | Description |
|--------|-------------|
| `--token <TOKEN>` | Use this token for this run (still written to config if login succeeds). |
| `--config <PATH>` | Config file path (default: see below). |
| `--api-base-url <URL>` | API base URL (default: `https://api.fluxer.app/v1`). |
| `--logout` | Clear stored token from config and exit. |

## Config file

Default path (unless you pass `--config`): **`…/fluxer-tui/config.toml`** under the OS config directory (on Linux this is usually **`~/.config/fluxer-tui/config.toml`**).

It stores `api_base_url`, `token`, `last_server_id`, and `last_channel_id` so the client can restore your last place, plus the `[ui]`, `[media]` and `[console]` settings described below.

## Pictures in chat

On a terminal that can draw pictures (sixel, kitty, iTerm2) and on the
console, the message pane looks like the web app:

- **Profile pictures** sit to the left of each message, two rows tall; a
  user without one gets the web app's default avatar. They are round on
  the console, kitty and iTerm2; sixel has no transparency, so there they
  are round only with the Fluxer theme (whose background is known) and
  square with the terminal theme.
- **Pictures, GIFs and video posters** are shown under the message as
  previews: scaled to fit about a third of the pane, never larger than the
  original. GIFs from the picker (KLIPY, Tenor) and animated WebP play; in
  performance mode they stay on their first frame. **Ctrl+O** on a selected
  message still opens the full-size picture or GIF, videos externally, and
  plays audio (see "Audio").

Previews are asked from Fluxer's media proxy already scaled to the size
they are drawn at, so a 4000-pixel photo costs a few kilobytes. Only the
pictures on screen are fetched (a few at a time); what is decoded stays in
memory within a budget and the least recently drawn go first; the
downloaded bytes are kept on disk under `~/.cache/fluxer-tui/media` within
a size cap, the oldest files evicted first. Nothing else is written.

```toml
[ui]
inline_media = true   # pictures and GIFs under messages
avatars = true        # profile pictures beside messages

[media]
disk_cache_mb = 64    # 0 turns the disk cache off
memory_cache_mb = 64
audio_player = ""     # see "Audio" below
```

Both `[ui]` switches are also in the settings overlay (**F2**). A picture
cut by the pane's edge shows the part that is on screen (on sixel, whole
six-pixel bands of it).

Scrolling is cheap on a terminal: when the pane merely scrolled, the
terminal is asked to shift those rows itself (pictures move with them, as
foot, kitty and xterm do), and only the rows that came into view are
sent. Keys that arrive while a frame is drawn are handled together, so a
held key scrolls as fast as the terminal keeps up.

## Audio

**Ctrl+O** on a message with an audio attachment (a voice message, an
mp3, ...) plays it; **Ctrl+O** on it again stops. The client does not
decode audio itself: the file is piped to a program that does, with no
terminal of its own, so it works the same in a terminal emulator and on a
Linux console. With nothing configured the first of these on PATH is
used, each told to play audio only and read stdin:

```
mpv --no-video --no-terminal --really-quiet -
ffplay -nodisp -autoexit -loglevel error -
pw-play -
paplay
aplay -q
```

Any other program that reads the audio from stdin does too:

```toml
[media]
audio_player = "sox -q -t mp3 - -d"
```

The status line shows what plays. Audio attachments are listed with ♪
and their length.

## Attaching files

Any kind of file can go with a message, up to ten at a time:

- **Ctrl+F**, or `/attach` on its own, opens a file picker: directories
  first, type to filter (a leading dot shows dotfiles), **Enter** opens a
  directory or attaches the file, **←** or **Backspace** on an empty
  filter goes up. What is under the cursor is described on the right,
  with a preview when it is a picture or a video.
- `/attach ~/path/to/file` attaches by path.
- **Ctrl+V** attaches what is on the clipboard: an image, or the files
  copied in a file manager (`wl-paste` or `xclip` do the reading).

Staged files show above the text you type, pictures and videos as
thumbnails where the terminal draws pictures, everything else by name and
size. **Ctrl+O** shows the last staged picture or video full size,
**Ctrl+X** drops it, **Enter** sends text and files together. A video's
preview is its first frame, which `ffmpeg` on PATH provides; without it
videos are listed by name.

## Notifications

A direct message, or a message that mentions you (directly, through one
of your roles, or with @everyone unless you suppress those), is announced
outside the client by a program that does that:

- on a desktop, libnotify's `notify-send`;
- on a Linux console, GNU `mail`, when you choose it: the message is
  mailed to you locally, so the console's "You have new mail" is the
  notification, and `mail` reads it.

The **Notifications** row in the settings overlay (**F2**) picks the
mode: **Auto** (the default: notify-send where there is a display,
nothing elsewhere), **Desktop**, **Mail** or **Off**. Mail is never
picked on its own, since it puts messages in your mailbox: set the mode
to Mail for the console. The config file has the rest:

```toml
[ui]
notifications = "auto"      # auto | desktop | mail (opt in) | off
notify_mail_to = ""         # recipient; the login user when empty
notify_mail_command = ""    # the mail program; "mail" when empty
notify_desktop_command = "" # the desktop program; "notify-send" when empty
notify_all_messages = false # also every message in channels set to all messages
```

Nothing is announced for your own messages, and a channel's own
notification settings (muted, mentions only) are respected. A program
that cannot be run is reported on the status line: `notify-send` comes
with libnotify and needs a notification daemon (mako, dunst, ...) to
show anything; `mail` comes with GNU mailutils and needs local mail
delivery.

## Interface overview

The UI has four **focus** areas, cycled with **Tab** / **Shift+Tab** (or **h**/**l** / **Left**/**Right**):

1. **Servers** - guild list and Direct Messages (`@me`).
2. **Channels** - channels for the selected server.
3. **Messages** - message history for the selected channel.
4. **Input** - compose box (text channels only, when you have permission to send).

A status line at the top shows gateway state, errors, and hints.


## Keyboard shortcuts

### Global (not typing in the input box)

| Key | Action |
|-----|--------|
| **Tab** | Next focus (Servers → Channels → Messages → Input → …). |
| **Shift+Tab** | Previous focus. |
| **Left** / **h** | Previous focus. |
| **Right** / **l** | Next focus. |
| **i** | Jump to **Input** (text channel, if you can send). |
| **Enter** | On a **link** channel: open URL in browser. On a **text** channel: jump to **Input**. |
| **Esc** | Clear message selection; focus **Channels**. |
| **↑** / **k** | Move selection / scroll (depends on focus; see below). |
| **↓** / **j** | Move selection / scroll (depends on focus). |
| **PageUp** | Scroll message list up (larger step). |
| **PageDown** | Scroll message list down (larger step). |
| **Ctrl+N** / **Ctrl+P** | Next / previous **text** channel (wraps; works from input too unless a popup is open). |
| **Ctrl+K** | Open **channel picker** (type to filter, **Enter** to jump). |
| **Alt+A** | Jump to the **next channel** (after current) that has **unread** or **mention** badges; wraps. |
| **F1** | **Keybindings** overlay - **↑** / **↓** / **PgUp** / **PgDn** scroll when it does not fit (**Esc** / **Enter** / **q** to close). |
| **Ctrl+H** | Same overlay when focus is **not** the message input (in input, **Ctrl+H** / **Ctrl+Backspace** delete the previous word). |
| **R** | **Refresh** active channel messages and guild metadata used for loads (clears local message cache for the channel and resets fetch/backoff state for the current guild). |
| **Ctrl+C** | Quit. |
| **Ctrl+L** | Log out (clear intent flag) and quit - token is cleared when the process exits cleanly after this. |
| **q** | Quit. |

### When focus is **Servers**

| Key | Action |
|-----|--------|
| **↑** / **k** | Previous server / DM entry. |
| **↓** / **j** | Next server / DM entry. |

### When focus is **Channels**

| Key | Action |
|-----|--------|
| **↑** / **k** | Previous channel. |
| **↓** / **j** | Next channel. |

Changing channels marks read state for the new channel when applicable.

### When focus is **Messages**

| Key | Action |
|-----|--------|
| **↑** / **k** | With **message select mode** on: previous message. Otherwise: scroll up a few lines. |
| **↓** / **j** | With **message select mode** on: next message. Otherwise: scroll down a few lines. |
| **s** | **Select** mode: select the latest message (start of thread for reply / react / forward). |
| **G** | Jump to the **newest** message (also from the Servers and Channels focus). While you are scrolled up, the view stays on the message you are reading as new messages arrive or older ones load. |
| **r** | **Reply** to the selected message (only in select mode). Moves focus to **Input** with reply state set. |
| **p** | **Profile** of the selected message's author: name, badges, pronouns, bio, when they joined Fluxer and the community, roles, connections, mutual communities and friends. **↑** / **↓** scroll; **p** again shows the profile picture full size; **Esc** / **q** close. |
| **e** | **React**: pick an emoji (**Enter** sends the reaction via API; **Esc** cancels). |
| **f** | **Forward**: optional note, switch target channel (**Ctrl+K** or list), **Enter** to send (reference type forward). |
| **Ctrl+E** | **Edit** the selected message (your messages only; **Enter** in input to save, **Esc** to cancel). |
| **Ctrl+D** | **Delete** the selected message (yours, or with **Manage Messages**). |
| **[** | Load **older messages** (prepends history; repeat until exhausted). |

Edited messages show **(edited)** in dim italics after the timestamp when the API supplies `edited_timestamp` (including live **MESSAGE_UPDATE** from the gateway).

### When focus is **Input**

| Key | Action |
|-----|--------|
| **Enter** | Send message, save **edit**, or forward with reference only. Long lines **wrap** and the input bar **grows** with the text. |
| **↑** | Leave **Input** and focus **Messages**. |
| **Backspace** | Delete character. |
| **Ctrl+Backspace** / **Ctrl+H** | Delete the previous whitespace-separated word. |
| **Ctrl+U** | Clear the whole input line. |
| **Esc** | If replying/forwarding, cancel; if picking a reaction, cancel; otherwise leave **Input** and focus **Channels**. |
| **:** (colon) | Start **custom emoji** autocomplete (server emojis + unicode picker). |
| **@** | Start **@mention** autocomplete (users/roles in guilds; DMs use recipients). Triggers loading full member list from the API only when needed. |

Plain letters (without **Ctrl**) are inserted into the message, except where autocomplete consumes them.

### @mention autocomplete (while open)

| Key | Action |
|-----|--------|
| **↑** / **↓** | Previous / next suggestion. |
| **Tab** / **Enter** | Insert selected mention. |
| **Esc** | Close autocomplete. |
| **Backspace** | Edit text; filter updates. |
| **Any character** | Type to filter (unless **Ctrl**). |

### Emoji autocomplete (while open)

| Key | Action |
|-----|--------|
| **↑** / **↓** | Previous / next emoji. |
| **Tab** / **Enter** | Insert selected emoji into the message, **or** confirm **reaction** when **e** flow is active. |
| **Esc** | Close autocomplete. |
| **Backspace** | Edit; filter updates. |
| **Any character** | Type to filter (unless **Ctrl**). |

---

## Tips

- **Capital R** is refresh; **lowercase r** in message select mode is reply.
- Message **select mode** is only active after **s** in the **Messages** focus.

## Console mode (Linux VT, no terminal emulator)

On a plain Linux virtual console there is no terminal emulator to draw
pictures, so fluxer-tui paints its own screen through DRM/KMS: text with a
real font, colour emoji, custom emoji, animated emoji, image previews and
GIFs, all on the console. It switches on by itself when `TERM=linux` and
stdin is a VT (a getty login on tty7, say), and needs:

- access to the DRM device: membership of the `video` group, or an active
  logind session on that VT (which grants it);
- a text font and an emoji font, found through `fc-match monospace` and
  `fc-match emoji` unless set explicitly.

Keys come from the VT as usual. Switching VTs (Alt+Fn) hands the display
over and back. Settings, all optional:

```toml
[console]
mode = "auto"        # auto | always | never
drm_device = ""      # default /dev/dri/card0
font = ""            # path to a TTF/OTF; default: fc-match monospace
bold_font = ""       # default: fc-match monospace:bold
emoji_font = ""      # default: fc-match emoji (e.g. Noto Color Emoji)
font_px = 28         # text size in pixels
```

`FLUXER_TUI_CONSOLE=never|always` overrides the mode for one run, and
`FLUXER_TUI_CONSOLE=dump:/some/dir:1280x720` renders frames to PPM files
instead of a display, which is how the console renderer is tested.

## Known issues & TODOs

- **Voice** is view-only: the client shows who is in a voice channel but
  cannot join, transmit or hear. Fluxer's voice runs over WebRTC through
  LiveKit, which would mean a whole WebRTC stack in the client.
- Some communities answer the member list request with a gateway
  timeout (504) from the server's own member service. The client keeps
  the pages that arrived, says in plain words that the list is
  unavailable, and offers @mentions from the members it has seen.
- Open a **feature request** issue for anything you want that is not here yet.

## License

MIT
