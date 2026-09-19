//! Spotify playback, track streaming, and metadata fetching module.

use anyhow::{Context, Result};
use librespot::core::{
    SpotifyUri,
    cache::Cache,
    config::SessionConfig,
    session::Session,
    spotify_id::SpotifyId,
};
use librespot::metadata::{Metadata, Track, Playlist};
use librespot::playback::{
    audio_backend,
    config::{AudioFormat, Bitrate, PlayerConfig},
    mixer::NoOpVolume,
    player::Player,
};
use serde::Serialize;
use std::path::Path;

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
    let cache = Cache::new(Some(cache_dir.to_path_buf()), None, None, None)
        .context("Failed to initialize Spotify cache")?;

    let credentials = cache.credentials().context(
        "No saved Spotify credentials found. Please run `spotstream auth` first to pair your account.",
    )?;

    let session_config = SessionConfig::default();
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
    player.load(track_uri, true, 0);

    eprintln!("[spotstream] Streaming PCM s16le 44100Hz stereo to stdout...");
    player.await_end_of_track().await;

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
