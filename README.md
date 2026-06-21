# Serenya Bot

Serenya is a Rust-based, multi-guild Discord music bot designed for high performance, safety, and stability. 
Built using `serenity` and `poise`, this bot aims to be incredibly robust while delivering features suitable for large, bustling servers.

## Features

- **Slash + Prefix Commands**: Support for modern Discord interaction.
- **High-Fidelity Audio Architecture**: Highly optimized audio streaming bypassing memory bottlenecks, with support for Deezer, Apple Music, Spotify, YouTube, and SoundCloud.
- **Audio Filters**: 8D, Bassboost, Nightcore, Pitch, Speed tuning out of the box using FFmpeg.
- **Graceful Shutdown**: Safe teardown sequence via cancellation tokens to stop background tasks and write final database states to disk.
- **Atomic Persistence**: Thread-safe atomic file writes with `.tmp` and `.bak` backup strategies to prevent data corruption.
- **Strict Error Handling**: Custom typed domain errors (`SerenyaError`) propagated safely up to Poise boundaries.
- **Low Memory Footprint**: Tailored for minimal resource environments (runs easily on a 1GB VPS). **Note**: The **only** bottleneck for speed is the local execution of `yt-dlp` to extract direct media streams, which is network and CPU dependent.

## Tech Stack

- **Runtime**: Rust (latest stable), Tokio (async runtime)
- **Discord Integration**: Poise + Serenity
- **Voice / Audio**: Songbird (bypassing Symphonia for raw FFmpeg pipelines)
- **Serialization**: Serde, serde-saphyr (YAML)
- **Logging**: Tracing

## Available Commands

Serenya comes packed with essential and advanced playback tools:

### 🎵 Playback
- `/play [query/url]` - Plays a song or playlist from Spotify, Deezer, Apple Music, YouTube, or SoundCloud.
- `/search [query]` - Shows a dropdown of top search results from various providers to choose from.
- `/pause` - Pauses the current track.
- `/resume` - Resumes a paused track.
- `/stop` - Stops playback and clears the queue.

### 📋 Queue Management
- `/queue` - Displays the current server queue (paginated).
- `/skip` - Skips the currently playing track.
- `/previous` - Plays the previously played track.
- `/jump [position]` - Skips to a specific track in the queue.
- `/remove [position]` - Removes a specific track from the queue.
- `/move [from] [to]` - Moves a track from one position to another.
- `/clear` - Clears all tracks from the queue except the one currently playing.
- `/shuffle` - Shuffles the upcoming tracks in the queue.
- `/loop [mode]` - Changes the loop mode (`Off`, `Track`, `Queue`).

### 🎛️ Audio Effects
- `/8d [on|off]` - Toggles the 8D audio effect.
- `/bassboost [level]` - Toggles or sets the bassboost level.
- `/nightcore [on|off]` - Toggles the nightcore (speed + pitch) effect.
- `/speed [multiplier]` - Adjusts the playback speed.
- `/pitch [multiplier]` - Adjusts the playback pitch.
- `/filter clear` - Clears all active audio filters.

### ℹ️ Information & Utilities
- `/nowplaying` - Displays detailed information about the currently playing track.
- `/lyrics [query]` - Fetches lyrics for the current song or a specific query.
- `/songinfo` - Displays raw metadata for the current track.
- `/ping` - Checks the bot's latency and connection status.

### ⚙️ Settings
- `/announce [on|off]` - Toggles "Now Playing" announcements in the channel.
- `/stay_in_voice [on|off]` - Toggles whether the bot should stay in the voice channel when the queue ends.
- `/default_volume [0-100]` - Sets the default playback volume for the server.

## Getting Started

### Prerequisites

- Rust 1.85+
- Visual Studio Build Tools (on Windows) or GCC/Clang (on Linux)
- CMake
- `ffmpeg` and `yt-dlp` installed and present in the system `PATH`.
  - On Windows, you can optionally place the executables in a `bin/` subdirectory in the project root.
  - On Linux (e.g. VPS Ubuntu), install them globally via package managers (e.g., `apt update && apt install ffmpeg python3 python3-pip -y && pip3 install -U yt-dlp`) and ensure they are accessible.

### Configuration

Create a `config.yml` based on the `config.example.yml` template.
Ensure your `DISCORD_TOKEN` environment variable is set or place it directly into the config file.

#### Getting the Spotify `sp_dc` cookie
Serenya requires an `sp_dc` cookie to access Spotify's internal Web API (which bypasses rate limits and provides full metadata).
1. Open Google Chrome or Firefox in Incognito/Private mode.
2. Go to [Spotify Web Player](https://open.spotify.com/) and log in with a regular or dummy account.
3. Press `F12` to open Developer Tools.
4. Go to the **Application** tab (Chrome) or **Storage** tab (Firefox).
5. Expand **Cookies** on the left menu and select `https://open.spotify.com`.
6. Find the row with the Name `sp_dc`.
7. Copy the Value and paste it into `config.yml` under `spotify.sp_dc`. 

**Warning:** Do NOT share this cookie publicly or commit it to GitHub. It acts as an authentication token for your Spotify account.

#### Custom Emojis
Serenya uses custom emojis in its embed messages (under the `emojis` section in `config.yml`). 
To ensure the bot displays these emojis properly:
1. You must manually upload your custom emojis to a Discord server where the bot is a member.
2. Obtain the emoji format (e.g., `<:spotify:123456789>`) by typing `\:emoji_name:` in the chat.
3. Paste the formatted string into the `config.yml`. If the bot is not in the server where the emoji was uploaded, it will not be able to render it.

### Running the Bot

Run the bot using:

```bash
cargo run --release
```

### Verification & Benchmarking

To run the test suite:

```bash
cargo test
```

### Benchmark Results & Resource Usage

Serenya has been rigorously benchmarked using a custom Global Allocator Tracker to eliminate memory churn (allocations on the Heap) during intensive track searching and ranking. 

#### 1. Search Ranking Algorithms (10,000 iterations)
- **Jaro-Winkler Similarity**: 0 Bytes Allocated (Static Stack arrays) | ~12.20 ms execution time
- **Token Overlap**: 0 Bytes Allocated (Zero-allocation Iterators) | ~4.05 ms execution time

#### 2. System Resource Consumption (VPS 1 Core / 1GB RAM)
- **Idle State**: 
  - RAM: ~15MB - 20MB
  - CPU: 0%
- **Playing Music (Normal)**: 
  - RAM: ~25MB - 30MB
  - CPU: ~0.5% - 1%
- **Playing Music (with 8D Audio / Filters enabled)**: 
  - RAM: ~30MB - 35MB
  - CPU: ~1% - 3% (Handled natively by FFmpeg stream piping, rust process remains near 0%)
- **Resolving Audio / Switching Qualities**: 
  - RAM: Minor spike (~5MB temporary)
  - CPU: Short spike (~3% - 5%) during `yt-dlp` stream metadata extraction, dropping back to <1% immediately.

The bot can serve thousands of concurrent users with effectively zero memory leaks and highly optimized background tasks.
