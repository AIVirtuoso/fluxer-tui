use crate::api::client::{ApiError, FluxerHttpClient};
use crate::api::types::UserPrivateResponse;
use crate::config::AppConfig;
use anyhow::{Context, Result, bail};
use reqwest::StatusCode;
use std::io::{self, Write};
use std::process::Command;
use tokio::time::{Duration, sleep};

pub struct AuthContext {
    pub token: String,
    pub me: UserPrivateResponse,
}

pub async fn ensure_auth(
    base_client: &FluxerHttpClient,
    config: &mut AppConfig,
    cli_token: Option<String>,
    webapp_url: &str,
) -> Result<AuthContext> {
    if let Some(token) = cli_token.or_else(|| config.token.clone()) {
        let client = base_client.with_token(token.clone());
        if let Ok(me) = client.current_user().await {
            config.token = Some(token.clone());
            return Ok(AuthContext { token, me });
        }
        eprintln!("Stored token was rejected, starting browser login.");
    }

    let handoff = base_client
        .handoff_initiate()
        .await
        .context("failed to initiate browser handoff")?;

    let code = &handoff.code;
    let poll_secret = handoff.poll_secret.as_deref();
    let formatted = if code.len() == 8 {
        format!("{}-{}", &code[..4], &code[4..])
    } else {
        code.clone()
    };

    copy_to_clipboard(&formatted);

    eprintln!();
    eprintln!("  Your login code: {formatted}");
    eprintln!("  (copied to clipboard)");
    eprintln!();
    eprintln!("  Opening your browser to complete login...");
    eprintln!("  If the browser doesn't open, go to:");
    eprintln!("  {webapp_url}/login?desktop_handoff=1");
    eprintln!();

    let login_url = format!("{webapp_url}/login?desktop_handoff=1");
    let _ = open_url(&login_url);

    eprint!("  Waiting for browser login");
    io::stderr().flush().ok();

    let max_attempts = 150; // 5 minutes at 2s intervals
    let mut last_error: Option<String> = None;
    for _ in 0..max_attempts {
        sleep(Duration::from_secs(2)).await;
        eprint!(".");
        io::stderr().flush().ok();

        match base_client.handoff_status(code, poll_secret).await {
            Ok(status) if status.status == "completed" => {
                eprintln!(" done!");
                let token = status
                    .token
                    .ok_or_else(|| anyhow::anyhow!("handoff completed without a token"))?;

                let me = base_client
                    .with_token(token.clone())
                    .current_user()
                    .await
                    .context("handoff succeeded but user verification failed")?;

                config.token = Some(token.clone());
                return Ok(AuthContext { token, me });
            }
            Ok(status) if status.status == "expired" => {
                eprintln!();
                bail!("login code expired, please try again");
            }
            Ok(_) => continue,
            Err(err) if is_handoff_lockout(&err) => {
                eprintln!();
                bail!("{}", LOCKOUT_EXPLANATION);
            }
            Err(err) => {
                // Surface a changed error once instead of dotting silently
                // for five minutes.
                let text = format!("{err:#}");
                if last_error.as_deref() != Some(text.as_str()) {
                    eprintln!();
                    eprintln!("  status check failed: {text}");
                    eprint!("  Waiting for browser login");
                    last_error = Some(text);
                }
                continue;
            }
        }
    }

    eprintln!();
    bail!("timed out waiting for browser login")
}

/// What the server means by 400 INVALID_HANDOFF_CODE on a status poll for
/// a code it has just issued: not the code, which is well-formed, but a
/// lockout of this IP address. The API counts failed handoff attempts per
/// client IP address and refuses every handoff request for 15 minutes
/// after the fifth failure. Polling does not add to the count, so waiting
/// is what helps.
const LOCKOUT_EXPLANATION: &str = "\
browser login is blocked from this IP address for now.
  The server answered INVALID_HANDOFF_CODE for a code it had just issued,
  which it does after five failed handoff attempts from one IP address
  within 15 minutes: a code typed wrongly on the login page, a code from
  a fluxer-tui that had already exited, or an older fluxer-tui polling
  without the poll secret. Wait 15 minutes without trying again (every
  wrong attempt starts the 15 minutes over), then run fluxer-tui once more
  and enter the code exactly as shown.";

/// True for the status poll's 400 INVALID_HANDOFF_CODE, which for a code
/// the server just issued can only be the per-address lockout.
fn is_handoff_lockout(err: &anyhow::Error) -> bool {
    matches!(
        err.downcast_ref::<ApiError>(),
        Some(ApiError::Response { status, code: Some(code), .. })
            if *status == StatusCode::BAD_REQUEST && code == "INVALID_HANDOFF_CODE"
    )
}

fn copy_to_clipboard(text: &str) {
    // try wayland first, then X11 idk other methods so that will probably have to be improved :)
    let attempts: &[(&str, &[&str])] = &[
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ];

    for (cmd, args) in attempts {
        if let Ok(mut child) = Command::new(cmd)
            .args(*args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            if let Some(stdin) = child.stdin.as_mut() {
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = child.wait();
            return;
        }
    }
}

fn open_url(url: &str) -> Result<()> {
    #[cfg(target_os = "linux")]
    let cmd = "xdg-open";
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(target_os = "windows")]
    let cmd = "start";

    Command::new(cmd)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("failed to open browser")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn api(status: StatusCode, code: Option<&str>) -> anyhow::Error {
        ApiError::Response {
            status,
            code: code.map(str::to_string),
            message: "Invalid handoff code.".into(),
            body: Value::Null,
        }
        .into()
    }

    #[test]
    fn only_the_invalid_code_answer_to_a_poll_is_the_lockout() {
        assert!(is_handoff_lockout(&api(
            StatusCode::BAD_REQUEST,
            Some("INVALID_HANDOFF_CODE")
        )));
        assert!(!is_handoff_lockout(&api(
            StatusCode::BAD_REQUEST,
            Some("INVALID_FORM_BODY")
        )));
        assert!(!is_handoff_lockout(&api(
            StatusCode::TOO_MANY_REQUESTS,
            Some("RATE_LIMITED")
        )));
        assert!(!is_handoff_lockout(&api(StatusCode::BAD_REQUEST, None)));
        assert!(!is_handoff_lockout(&anyhow::anyhow!("connection reset")));
        assert!(LOCKOUT_EXPLANATION.contains("15 minutes"));
    }
}
