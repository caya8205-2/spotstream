//! # spotstream
//!
//! High-performance Spotify streaming and metadata extraction adapter.
//! Built on `librespot` with support for direct 320kbps Vorbis decoding to raw PCM,
//! RFC 8628 OAuth device pairing (`spotify.com/pair`), and personalized Mercury playlist fetching.

pub mod auth;
pub mod player;

pub use auth::{DeviceAuthResponse, poll_and_save, request_pairing_code, run_device_pairing};
pub use player::{
    PlaylistInfo, PlaylistItemInfo, TrackInfo, fetch_playlist_info, fetch_track_info, get_session,
    parse_playlist_id, parse_track_id, stream_track,
};
