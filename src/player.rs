//! Spotify playback, track streaming, and metadata fetching module.

use anyhow::{Context, Result, bail};
use librespot::core::{
    SpotifyUri,
    cache::Cache,
    config::SessionConfig,
    session::Session,
    spotify_id::SpotifyId,
};
use librespot::metadata::{Metadata, Track, Playlist};
use librespot::playback::{
    audio_backend::{self, Sink, SinkAsBytes, SinkError, SinkResult},
    config::{AudioFormat, Bitrate, PlayerConfig},
    convert::Converter,
    decoder::AudioPacket,
    mixer::NoOpVolume,
    player::{Player, PlayerEvent},
};
use serde::Serialize;
use std::path::Path;
use std::sync::mpsc::{channel, Sender};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

/// Parse a track ID from raw base62 string, Spotify URI (`spotify:track:...`), or web URL.
pub fn parse_track_id(input: &str) -> Result<SpotifyId> {
    let clean = input.trim();
    let id_str = if clean.starts_with("spotify:track:") {
        clean.strip_prefix("spotify:track:").unwrap()
    } else if let Some(idx) = clean.find("/track/") {
        let after = &clean[idx + 7..];
        after.split('?').next().unwrap_or(after)
    } else {
        clean
    };

    SpotifyId::from_base62(id_str)
        .map_err(|e| anyhow::anyhow!("Invalid Spotify track ID or URI '{id_str}': {e}"))
}

/// Parse a playlist ID from raw base62 string, Spotify URI (`spotify:playlist:...`), or web URL.
pub fn parse_playlist_id(input: &str) -> Result<SpotifyId> {
    let clean = input.trim();
    let id_str = if clean.starts_with("spotify:playlist:") {
        clean.strip_prefix("spotify:playlist:").unwrap()
    } else if let Some(idx) = clean.find("/playlist/") {
        let after = &clean[idx + 10..];
        after.split('?').next().unwrap_or(after)
    } else {
        clean
    };

    SpotifyId::from_base62(id_str)
        .map_err(|e| anyhow::anyhow!("Invalid Spotify playlist ID or URI '{id_str}': {e}"))
}

/// Initialize a connected Spotify session using cached credentials.
pub async fn get_session(cache_dir: &Path) -> Result<Session> {
    let files_dir = cache_dir.join("files");
    let cache = Cache::new(Some(cache_dir.to_path_buf()), None, Some(files_dir), None)
        .context("Failed to initialize Spotify cache")?;

    let credentials = cache.credentials().context(
        "No saved Spotify credentials found. Please run `spotstream auth` first to pair your account.",
    )?;

    // Use a stable device_id derived from machine/username so Spotify doesn't treat every run as a new login
    let mut session_config = SessionConfig::default();
    let cred_path = cache_dir.join("credentials.json");
    if let Ok(meta) = std::fs::metadata(&cred_path) {
        if let Ok(modified) = meta.modified() {
            let dur = modified.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
            session_config.device_id = format!("spotstream-{:x}", dur);
        }
    }
    let session = Session::new(session_config, Some(cache));

    session
        .connect(credentials, false)
        .await
        .context("Failed to connect to Spotify AccessPoint. Check your network or run `spotstream auth` again.")?;

    Ok(session)
}

/// Track metadata information container.
#[derive(Serialize)]
pub struct TrackInfo {
    /// Base62 Spotify track ID
    pub id: String,
    /// Track title
    pub title: String,
    /// List of artist names
    pub artists: Vec<String>,
    /// Album name
    pub album: String,
    /// Track duration in milliseconds
    pub duration_ms: i32,
}

/// Fetch track metadata by ID, URI, or URL.
pub async fn fetch_track_info(cache_dir: &Path, track_input: &str) -> Result<TrackInfo> {
    let track_id = parse_track_id(track_input)?;
    let session = get_session(cache_dir).await?;

    let track_uri = SpotifyUri::Track { id: track_id };
    let track = Track::get(&session, &track_uri)
        .await
        .context("Failed to fetch track metadata from Spotify")?;

    let artists = track.artists.iter().map(|a| a.name.clone()).collect();

    let base62_id = track_id.to_base62().unwrap_or_else(|_| "unknown".to_string());
    let info = TrackInfo {
        id: base62_id,
        title: track.name,
        artists,
        album: track.album.name,
        duration_ms: track.duration,
    };

    session.shutdown();
    Ok(info)
}

/// Stream decoded audio (PCM s16le 44100Hz stereo) directly to standard output.
pub async fn stream_track(cache_dir: &Path, track_input: &str) -> Result<()> {
    let track_id = parse_track_id(track_input)?;
    let session = get_session(cache_dir).await?;

    let player_config = PlayerConfig {
        bitrate: Bitrate::Bitrate320,
        gapless: false,
        ..Default::default()
    };

    let audio_format = AudioFormat::S16;
    let backend = audio_backend::find(Some("pipe".to_string()))
        .context("Pipe audio backend not available")?;

    let player = Player::new(player_config, session.clone(), Box::new(NoOpVolume), move || {
        backend(None, audio_format)
    });

    let track_uri = SpotifyUri::Track { id: track_id };
    let base62_id = track_id.to_base62().unwrap_or_else(|_| "unknown".to_string());
    eprintln!("[spotstream] Loading track {}...", base62_id);

    let mut event_channel = player.get_player_event_channel();
    player.load(track_uri, true, 0);

    eprintln!("[spotstream] Streaming PCM s16le 44100Hz stereo to stdout...");

    // Protect against hanging indefinitely if track is unavailable, region-blocked, or failed
    let startup_timeout = Duration::from_secs(15);
    let mut playback_started = false;

    loop {
        let event = if !playback_started {
            match tokio::time::timeout(startup_timeout, event_channel.recv()).await {
                Ok(Some(ev)) => ev,
                Ok(None) => bail!("Player event channel closed before playback started"),
                Err(_) => {
                    player.stop();
                    session.shutdown();
                    bail!("Timeout waiting for playback to start (track may be unavailable or network dropped)");
                }
            }
        } else {
            match event_channel.recv().await {
                Some(ev) => ev,
                None => break,
            }
        };

        match event {
            PlayerEvent::Playing { .. } => {
                playback_started = true;
            }
            PlayerEvent::Unavailable { .. } => {
                player.stop();
                session.shutdown();
                bail!("Track is unavailable (region-locked, removed, or failed to decrypt)");
            }
            PlayerEvent::EndOfTrack { .. } | PlayerEvent::Stopped { .. } => {
                break;
            }
            _ => {}
        }
    }

    eprintln!("[spotstream] Finished streaming track.");
    session.shutdown();
    Ok(())
}

/// Item inside a playlist track list.
#[derive(Serialize)]
pub struct PlaylistItemInfo {
    /// Base62 Spotify track ID
    pub id: String,
    /// Canonical Spotify track URI (`spotify:track:...`)
    pub uri: String,
}

/// Playlist metadata and list of tracks.
#[derive(Serialize)]
pub struct PlaylistInfo {
    /// Playlist title
    pub name: String,
    /// Playlist description
    pub description: String,
    /// List of contained tracks
    pub tracks: Vec<PlaylistItemInfo>,
}

/// Fetch playlist metadata and track IDs (supports personalized Daily Mixes).
pub async fn fetch_playlist_info(cache_dir: &Path, playlist_input: &str) -> Result<PlaylistInfo> {
    let playlist_id = parse_playlist_id(playlist_input)?;
    let session = get_session(cache_dir).await?;

    let playlist_uri = SpotifyUri::Playlist {
        id: playlist_id,
        user: None,
    };

    let playlist = Playlist::get(&session, &playlist_uri)
        .await
        .context("Failed to fetch playlist metadata from Spotify")?;

    let mut tracks = Vec::new();
    for item in playlist.contents.items.0 {
        if let SpotifyUri::Track { id } = item.id {
            let base62 = id.to_base62().unwrap_or_else(|_| "unknown".to_string());
            tracks.push(PlaylistItemInfo {
                id: base62.clone(),
                uri: format!("spotify:track:{base62}"),
            });
        }
    }

    let info = PlaylistInfo {
        name: playlist.attributes.name,
        description: playlist.attributes.description,
        tracks,
    };

    session.shutdown();
    Ok(info)
}

struct ChannelSink {
    sender: Sender<Vec<u8>>,
    #[allow(dead_code)]
    format: AudioFormat,
}

impl Sink for ChannelSink {
    fn write(&mut self, packet: AudioPacket, converter: &mut Converter) -> SinkResult<()> {
        match packet {
            AudioPacket::Samples(samples) => {
                let samples_s16 = converter.f64_to_s16(&samples);
                let bytes = unsafe {
                    std::slice::from_raw_parts(
                        samples_s16.as_ptr() as *const u8,
                        samples_s16.len() * std::mem::size_of::<i16>(),
                    )
                };
                self.write_bytes(bytes)
            }
            AudioPacket::Raw(samples) => self.write_bytes(&samples),
        }
    }
}

impl SinkAsBytes for ChannelSink {
    fn write_bytes(&mut self, data: &[u8]) -> SinkResult<()> {
        self.sender
            .send(data.to_vec())
            .map_err(|e| SinkError::OnWrite(e.to_string()))
    }
}

/// Run a persistent Spotify streaming daemon that keeps the Spotify AccessPoint session warm in RAM.
/// Serves decoded PCM s16le 44100Hz stereo directly over a local TCP socket for sub-second playback start.
pub async fn run_daemon(cache_dir: &Path, port: u16) -> Result<()> {
    let session = get_session(cache_dir).await?;
    let listener = TcpListener::bind(format!("127.0.0.1:{}", port))
        .await
        .with_context(|| format!("Failed to bind spotstream daemon to 127.0.0.1:{}", port))?;

    // Signal to parent process that daemon is warm and listening
    println!("{{\"status\":\"ready\",\"port\":{}}}", port);

    loop {
        let (mut socket, _) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                eprintln!("[spotstream daemon] Accept error: {e}");
                continue;
            }
        };

        let session = session.clone();
        tokio::spawn(async move {
            let (reader, mut writer) = socket.split();
            let mut buf_reader = BufReader::new(reader);
            let mut line = String::new();

            if buf_reader.read_line(&mut line).await.is_err() || line.trim().is_empty() {
                return;
            }

            let line_trimmed = line.trim();
            let track_input = if let Some(stripped) = line_trimmed.strip_prefix("STREAM ") {
                stripped.trim()
            } else {
                line_trimmed
            };

            let track_id = match parse_track_id(track_input) {
                Ok(id) => id,
                Err(e) => {
                    eprintln!("[spotstream daemon] Invalid track ID '{track_input}': {e}");
                    return;
                }
            };

            let (tx, rx) = channel();
            let player_config = PlayerConfig {
                bitrate: Bitrate::Bitrate320,
                gapless: false,
                ..Default::default()
            };

            let player = Player::new(player_config, session.clone(), Box::new(NoOpVolume), move || {
                Box::new(ChannelSink {
                    sender: tx,
                    format: AudioFormat::S16,
                })
            });

            let track_uri = SpotifyUri::Track { id: track_id };
            player.load(track_uri, true, 0);

            // Channel bridging thread: librespot callback thread -> tokio async task
            let (pcm_tx, mut pcm_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
            std::thread::spawn(move || {
                while let Ok(bytes) = rx.recv() {
                    if pcm_tx.blocking_send(bytes).is_err() {
                        break;
                    }
                }
            });

            while let Some(bytes) = pcm_rx.recv().await {
                if writer.write_all(&bytes).await.is_err() {
                    break;
                }
            }

            player.stop();
        });
    }
}
