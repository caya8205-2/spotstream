use anyhow::{Context, Result};
use librespot::core::{
    SpotifyUri,
    cache::Cache,
    config::SessionConfig,
    session::Session,
    spotify_id::SpotifyId,
};
use librespot::metadata::{Metadata, Track};
use librespot::playback::{
    audio_backend,
    config::{AudioFormat, Bitrate, PlayerConfig},
    mixer::NoOpVolume,
    player::Player,
};
use serde::Serialize;
use std::path::Path;

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

#[derive(Serialize)]
pub struct TrackInfo {
    pub id: String,
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub duration_ms: i32,
}

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

pub async fn stream_track(cache_dir: &Path, track_input: &str) -> Result<()> {
    let track_id = parse_track_id(track_input)?;
    let session = get_session(cache_dir).await?;

    let mut player_config = PlayerConfig::default();
    player_config.bitrate = Bitrate::Bitrate320;
    player_config.gapless = false;

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
