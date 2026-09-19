# spotstream

[![Rust](https://img.shields.io/badge/Rust-1.85+-DEA584?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![librespot](https://img.shields.io/badge/librespot-0.8.0-1DB954?logo=spotify&logoColor=white)](https://github.com/librespot-org/librespot)
[![License](https://img.shields.io/badge/License-MIT-white)](./LICENSE)

A lightweight CLI adapter built on `librespot` for direct Spotify audio streaming, metadata extraction, and playlist fetching. It outputs raw PCM directly to `stdout`, designed for Discord bots, media servers, and FFmpeg pipelines.

---

## Features

- **Direct Spotify Decryption**: Fetches audio keys via Spotify's Mercury/AP protocol and decrypts 320kbps Vorbis streams without YouTube conversion or scraping.
- **Raw PCM Pipe**: Emits decoded `s16le` 44100Hz stereo PCM directly to `stdout` for zero-latency FFmpeg transcoding.
- **RFC 8628 Device Pairing**: One-time authorization via `https://spotify.com/pair`. No plaintext credentials in configs.
- **Personalized Playlist Support**: Reads standard playlists as well as dynamic personalized playlists (Daily Mix, Made for You, Discover Weekly) via internal Mercury endpoints.
- **Machine-Readable CLI**: Supports JSON output for track info, playlist dumps, and headless OAuth pairing (`auth-code` / `auth-poll`).
- **Persistent Session Cache**: Automatically reuses saved tokens from `%LOCALAPPDATA%/spotstream/cache` (Windows) or `~/.local/share/spotstream/cache` (Linux/macOS).

---

## Installation

### Build from Source
```bash
git clone https://github.com/caya8205-2/spotstream.git
cd spotstream
cargo build --release
```
The compiled binary will be located at `target/release/spotstream.exe` (or `target/release/spotstream` on Unix).

---

## Usage

### 1. One-Time Authorization
```bash
spotstream auth
```
Opens `https://spotify.com/pair` in your browser with a generated user code. Approve the device on your Spotify Premium account to save reusable session credentials.

For headless / automated bot flows:
```bash
# Request pairing code as JSON
spotstream auth-code

# Poll until approved
spotstream auth-poll --device-code "<DEVICE_CODE>"
```

### 2. Check Status
```bash
spotstream status
```

### 3. Stream Audio to FFmpeg (Stdout Pipe)
```bash
# Pipe raw PCM into FFmpeg
spotstream stream "4cOdK2wGLETKBW3PvgPWqT" | ffmpeg -f s16le -ar 44100 -ac 2 -i pipe:0 -c:a libopus -b:a 128k -f ogg output.ogg
```

### 4. Fetch Track Metadata
```bash
spotstream info "4cOdK2wGLETKBW3PvgPWqT"
```

Output:
```json
{
  "id": "4cOdK2wGLETKBW3PvgPWqT",
  "title": "Never Gonna Give You Up",
  "artists": [
    "Rick Astley"
  ],
  "album": "Whenever You Need Somebody",
  "duration_ms": 213573
}
```

### 5. Fetch Playlist & Daily Mix Tracks
```bash
spotstream playlist "https://open.spotify.com/playlist/37i9dQZF1E38AosGk0ttnv"
```

Output:
```json
{
  "name": "Daily Mix 3",
  "description": "Kanaria, IRyS, しぐれうい and more",
  "tracks": [
    {
      "id": "22EHB5z2GwYNPA1wZ3LtL4",
      "uri": "spotify:track:22EHB5z2GwYNPA1wZ3LtL4"
    }
  ]
}
```

---

## Documentation

Generate and view documentation locally:
```bash
cargo doc --no-deps --open
```

---

## Requirements

- **Spotify Premium account**: Required by Spotify's Mercury audio key servers to decrypt full-length tracks.
