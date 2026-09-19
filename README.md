# spotstream

High-performance Spotify streaming CLI adapter for Discord bots, media servers, and audio pipelines. Built with Rust and `librespot`.

## Features
- **Direct Spotify Decryption**: Streams high quality (320kbps) audio directly from Spotify CDN without YouTube matching or scraping.
- **Fast Startup**: Decodes raw Vorbis audio and outputs PCM (`s16le`, 44.1kHz, stereo) straight to `stdout` for instant FFmpeg ingestion.
- **Device Authorization (RFC 8628)**: Easy one-time OAuth pairing via `https://spotify.com/pair`. No plaintext passwords in config files.
- **CLI & Bot Automation Modes**: Supports interactive terminal pairing (`auth`) and machine-readable JSON pairing (`auth-code` / `auth-poll`).
- **Reusable Session Cache**: Session tokens stored securely in `%LOCALAPPDATA%/spotstream/cache` (or custom `--cache-dir`).

## Usage

### 1. Authorize (One-Time)
```bash
spotstream auth
```
This requests a pairing code from Spotify, opens `https://spotify.com/pair` in your browser, and waits for your confirmation.

Or for headless / bot automation:
```bash
# Request code as JSON
spotstream auth-code

# Poll until user approves
spotstream auth-poll --device-code "<DEVICE_CODE>"
```

### 2. Check Status
```bash
spotstream status
```

### 3. Stream a Track (Stdout Pipe)
```bash
# Pipe raw PCM into FFmpeg
spotstream stream "spotify:track:4cOdK2wGLETKBW3PvgPWqT" | ffmpeg -f s16le -ar 44100 -ac 2 -i pipe:0 output.ogg
```

### 4. Fetch Track Info
```bash
spotstream info "4cOdK2wGLETKBW3PvgPWqT"
```

## Requirements
- Spotify Premium account (required by Spotify's Mercury audio key servers).
