//! Handing a voice call's audio to a program on PATH.
//!
//! Fluxer carries voice media over LiveKit: the gateway hands a session
//! a signalling URL and a token, and everything after that is the
//! LiveKit protocol — WebRTC, with ICE, DTLS-SRTP and an Opus codec.
//!
//! This client does not speak any of that, on purpose. Bringing it in
//! would mean libwebrtc, a C++ toolchain in the build, and a great deal
//! of code that has nothing to do with drawing a terminal. So voice
//! follows the same division as the audio player and the notification
//! sender: the client decides — joins, leaves, mutes, keeps the
//! bookkeeping, knows who is in the channel — and a program named in
//! `[media] voice_command` carries the sound, given the URL and the
//! token the server issued.

use std::process::{Command, Stdio};

/// A voice grant, split into the three things a command line needs.
pub struct GrantParts<'a> {
    pub url: &'a str,
    pub token: &'a str,
    pub key: Option<&'a str>,
}

/// Fill `{url}`, `{token}` and `{key}` into a command template, and
/// split it into a program and its arguments.
///
/// Substitution happens after splitting, so a token can never introduce
/// a new argument however it is punctuated: whatever the server sent
/// lands in exactly one argv slot.
pub fn build_command(template: &str, grant: &GrantParts<'_>) -> Option<Vec<String>> {
    let parts: Vec<String> = template
        .split_whitespace()
        .map(|part| {
            part.replace("{url}", grant.url)
                .replace("{token}", grant.token)
                .replace("{key}", grant.key.unwrap_or(""))
        })
        .collect();
    if parts.is_empty() || parts[0].is_empty() {
        return None;
    }
    Some(parts)
}

/// Start the program. It is left to run on its own; leaving the channel
/// kills it, and its output goes nowhere, since a token must not end up
/// in a log and the terminal is busy being the UI.
pub fn spawn(argv: &[String]) -> std::io::Result<std::process::Child> {
    Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant<'a>(key: Option<&'a str>) -> GrantParts<'a> {
        GrantParts {
            url: "wss://voice.example/livekit",
            token: "eyJhbGciOi.tok.en",
            key,
        }
    }

    #[test]
    fn the_placeholders_are_filled_in() {
        let argv = build_command("livekit-cli join --url {url} --token {token}", &grant(None))
            .expect("a command");
        assert_eq!(
            argv,
            vec![
                "livekit-cli",
                "join",
                "--url",
                "wss://voice.example/livekit",
                "--token",
                "eyJhbGciOi.tok.en"
            ]
        );
    }

    #[test]
    fn a_missing_key_becomes_empty_rather_than_the_placeholder() {
        let argv = build_command("p --key {key}", &grant(None)).expect("a command");
        assert_eq!(argv, vec!["p", "--key", ""]);
        let argv = build_command("p --key {key}", &grant(Some("secret"))).expect("a command");
        assert_eq!(argv, vec!["p", "--key", "secret"]);
    }

    #[test]
    fn a_token_with_spaces_in_it_stays_one_argument() {
        // the template is split first and the values put in after, so
        // nothing the server sends can add an argument of its own
        let sneaky = GrantParts {
            url: "wss://x",
            token: "tok --publish-microphone /etc/passwd",
            key: None,
        };
        let argv = build_command("p --token {token} --end", &sneaky).expect("a command");
        assert_eq!(argv.len(), 4);
        assert_eq!(argv[2], "tok --publish-microphone /etc/passwd");
        assert_eq!(argv[3], "--end");
    }

    #[test]
    fn an_empty_template_is_no_command_at_all() {
        assert!(build_command("", &grant(None)).is_none());
        assert!(build_command("   ", &grant(None)).is_none());
    }
}
