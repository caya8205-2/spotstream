use anyhow::Result;
use clap::{Parser, Subcommand};
use serde::Serialize;
use std::path::PathBuf;

mod auth;
mod player;

#[derive(Parser)]
#[command(name = "spotstream")]
#[command(author = "Caya <caya8205@users.noreply.github.com>")]
#[command(version = "0.1.0")]
#[command(about = "High-performance Spotify streaming CLI adapter for Discord bots and audio pipelines", long_about = None)]
struct Cli {
    /// Custom cache directory for Spotify session credentials
    #[arg(short, long)]
    cache_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Interactive device authorization via browser pairing (spotify.com/pair)
    Auth,

    /// Request a new device pairing code and output JSON (for bot/GUI integration)
    AuthCode,

    /// Poll Spotify until user approves device code and save credentials
    AuthPoll {
        #[arg(long)]
        device_code: String,
        #[arg(long, default_value_t = 3600)]
        expires_in: u64,
        #[arg(long, default_value_t = 5)]
        interval: u64,
    },

    /// Check Spotify authentication status
    Status,

    /// Stream decoded audio (PCM s16le 44100Hz stereo) directly to stdout
    Stream {
        /// Spotify track ID, URI (spotify:track:...), or track URL
        track: String,
    },

    /// Retrieve track metadata as JSON
    Info {
        /// Spotify track ID, URI, or track URL
        track: String,
    },
}

#[derive(Serialize)]
struct StatusResult {
    authenticated: bool,
    cache_dir: String,
    credentials_file: String,
}

fn resolve_cache_dir(custom: Option<PathBuf>) -> PathBuf {
    if let Some(p) = custom {
        return p;
    }
    if let Ok(env_dir) = std::env::var("SPOTSTREAM_CACHE_DIR") {
        if !env_dir.trim().is_empty() {
            return PathBuf::from(env_dir.trim());
        }
    }
    if let Some(mut local_dir) = dirs::data_local_dir() {
        local_dir.push("spotstream");
        local_dir.push("cache");
        return local_dir;
    }
    PathBuf::from(".spotstream_cache")
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let cli = Cli::parse();
    let cache_dir = resolve_cache_dir(cli.cache_dir);

    match cli.command {
        Commands::Auth => {
            std::fs::create_dir_all(&cache_dir)?;
            auth::run_device_pairing(&cache_dir).await?;
        }
        Commands::AuthCode => {
            let data = auth::request_pairing_code().await?;
            println!("{}", serde_json::to_string_pretty(&data)?);
        }
        Commands::AuthPoll {
            device_code,
            expires_in,
            interval,
        } => {
            std::fs::create_dir_all(&cache_dir)?;
            auth::poll_and_save(&cache_dir, &device_code, expires_in, interval).await?;
            let status = StatusResult {
                authenticated: true,
                cache_dir: cache_dir.to_string_lossy().to_string(),
                credentials_file: cache_dir.join("credentials.json").to_string_lossy().to_string(),
            };
            println!("{}", serde_json::to_string_pretty(&status)?);
        }
        Commands::Status => {
            let cred_file = cache_dir.join("credentials.json");
            let authenticated = cred_file.is_file();
            let status = StatusResult {
                authenticated,
                cache_dir: cache_dir.to_string_lossy().to_string(),
                credentials_file: cred_file.to_string_lossy().to_string(),
            };
            println!("{}", serde_json::to_string_pretty(&status)?);
        }
        Commands::Stream { track } => {
            player::stream_track(&cache_dir, &track).await?;
        }
        Commands::Info { track } => {
            let info = player::fetch_track_info(&cache_dir, &track).await?;
            println!("{}", serde_json::to_string_pretty(&info)?);
        }
    }

    Ok(())
}
