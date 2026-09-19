//! Spotify Device Authorization flow implementation (RFC 8628).
//!
//! Provides headless one-time pairing via `https://spotify.com/pair`,
//! returning tokens and saving reusable credentials.

use anyhow::{Context, Result, bail};
use librespot::core::{authentication::Credentials, cache::Cache, config::SessionConfig, session::Session};
use serde::{Deserialize, Serialize};
use std::{path::Path, time::Duration};

const SPOTIFY_CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";
const DEVICE_AUTH_URL: &str = "https://accounts.spotify.com/oauth2/device/authorize";
const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
const SCOPES: &str = "streaming,user-read-email,user-read-private,user-library-read";

/// Device authorization response structure from Spotify's RFC 8628 endpoint.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct DeviceAuthResponse {
    /// Internal device code used for token polling
    pub device_code: String,
    /// 6-character user pairing code displayed to the user
    pub user_code: String,
    /// Verification URL (typically `https://spotify.com/pair`)
    pub verification_uri: String,
    /// Direct verification URL including pre-filled user code
    pub verification_uri_complete: Option<String>,
    /// Lifetime of authorization code in seconds
    pub expires_in: u64,
    /// Minimum polling interval required by Spotify in seconds
    #[serde(default = "default_interval")]
    pub interval: u64,
}

fn default_interval() -> u64 {
    5
}

#[derive(Deserialize, Debug)]
struct TokenSuccessResponse {
    access_token: String,
    #[allow(dead_code)]
    token_type: String,
    #[allow(dead_code)]
    expires_in: u64,
    #[allow(dead_code)]
    refresh_token: Option<String>,
}

#[derive(Deserialize, Debug)]
struct TokenErrorResponse {
    error: String,
    #[allow(dead_code)]
    error_description: Option<String>,
}

/// Request a new device pairing code and URL from Spotify's authorization server.
pub async fn request_pairing_code() -> Result<DeviceAuthResponse> {
    let client = reqwest::Client::new();
    let res = client
        .post(DEVICE_AUTH_URL)
        .form(&[
            ("client_id", SPOTIFY_CLIENT_ID),
            ("scope", SCOPES),
        ])
        .send()
        .await
        .context("Failed to request device authorization code")?;

    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        bail!("Spotify device authorize failed (HTTP {status}): {body}");
    }

    let auth_data: DeviceAuthResponse = res
        .json()
        .await
        .context("Failed to parse Spotify device authorize response")?;

    Ok(auth_data)
}

/// Poll Spotify until user approves the device pairing code, then connect and save session credentials.
pub async fn poll_and_save(
    cache_dir: &Path,
    device_code: &str,
    expires_in: u64,
    interval: u64,
) -> Result<()> {
    let client = reqwest::Client::new();
    let start_time = std::time::Instant::now();
    let timeout = Duration::from_secs(expires_in);
    let poll_interval = Duration::from_secs(interval.max(3));

    let access_token = loop {
        if start_time.elapsed() > timeout {
            bail!("Device authorization pairing timed out after {} seconds.", expires_in);
        }

        let token_res = client
            .post(TOKEN_URL)
            .form(&[
                ("client_id", SPOTIFY_CLIENT_ID),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", device_code),
            ])
            .send()
            .await;

        let token_res = match token_res {
            Ok(r) => r,
            Err(err) => {
                eprintln!("[spotstream] Network warning while polling: {err}");
                tokio::time::sleep(poll_interval).await;
                continue;
            }
        };

        let status = token_res.status();
        let bytes = token_res.bytes().await.context("Failed to read token response body")?;

        if status.is_success() {
            let success: TokenSuccessResponse = serde_json::from_slice(&bytes)
                .context("Failed to parse token response")?;
            break success.access_token;
        }

        if let Ok(err_data) = serde_json::from_slice::<TokenErrorResponse>(&bytes) {
            if err_data.error == "authorization_pending" {
                eprint!(".");
                let _ = std::io::Write::flush(&mut std::io::stderr());
                tokio::time::sleep(poll_interval).await;
                continue;
            } else if err_data.error == "slow_down" {
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            } else {
                bail!("Spotify authorization rejected: {}", err_data.error);
            }
        } else {
            let body_str = String::from_utf8_lossy(&bytes);
            bail!("Unexpected response from token endpoint (HTTP {status}): {body_str}");
        }
    };

    eprintln!("\n[spotstream] Authorization code approved by Spotify!");
    eprintln!("[spotstream] Connecting session to store reusable credentials...");

    let session_config = SessionConfig::default();
    let cache = Cache::new(Some(cache_dir.to_path_buf()), None, None, None)
        .context("Failed to initialize Spotify cache directory")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(cache_dir, std::fs::Permissions::from_mode(0o700));
    }

    let session = Session::new(session_config, Some(cache));
    let credentials = Credentials::with_access_token(access_token);

    session
        .connect(credentials, true)
        .await
        .context("Failed to connect to Spotify AP with access token")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let cred_file = cache_dir.join("credentials.json");
        if cred_file.is_file() {
            let _ = std::fs::set_permissions(&cred_file, std::fs::Permissions::from_mode(0o600));
        }
    }

    eprintln!("[spotstream] Reusable credentials successfully saved to: {}", cache_dir.display());
    session.shutdown();

    Ok(())
}

/// Run interactive device pairing by displaying the user code and opening the default browser.
pub async fn run_device_pairing(cache_dir: &Path) -> Result<()> {
    eprintln!("[spotstream] Requesting device authorization from Spotify...");
    let auth_data = request_pairing_code().await?;

    let pair_url = auth_data
        .verification_uri_complete
        .as_deref()
        .unwrap_or(&auth_data.verification_uri);

    eprintln!("\n=======================================================");
    eprintln!("  SPOTIFY DEVICE AUTHORIZATION (RFC 8628)");
    eprintln!("=======================================================");
    eprintln!("  Code: {}", auth_data.user_code);
    eprintln!("  Link: {pair_url}");
    eprintln!("-------------------------------------------------------");
    eprintln!("  Opening pairing link in your default browser...");
    eprintln!("  Log in with your Spotify Premium account and approve.");
    eprintln!("=======================================================\n");

    let _ = open::that_in_background(pair_url);

    poll_and_save(cache_dir, &auth_data.device_code, auth_data.expires_in, auth_data.interval).await
}
