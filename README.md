# yamusic-cli

> Lightweight background daemon and CLI client for Yandex Music in Rust with MPRIS2 support.

![Rust](https://img.shields.io/badge/rust-2021%20edition-orange.svg)
![Audio](https://img.shields.io/badge/audio-rodio%20%2F%20cpal-blue.svg)
![MPRIS](https://img.shields.io/badge/MPRIS2-D--Bus-blueviolet.svg)
![License](https://img.shields.io/badge/license-MIT-green.svg)

---

## Features

- **Daemon Architecture**: Lightweight background daemon communicating via Unix domain socket (`yamusic-cli.sock`). Low memory footprint (~20–30 MB RAM) and 0% CPU idle usage.
- **MPRIS2 (D-Bus) Integration**: Native support for `playerctl`, media keys (keyboard and headsets), lockscreen widgets, and status bars (Waybar, Polybar).
- **"My Wave" (Моя волна)**: Infinite personal recommendation stream powered by the Rotor algorithm with seamless background prefetching.
- **Disk Cache & Prefetching**:
  - Downloaded MP3 tracks are cached in `~/.cache/yamusic-cli/tracks/` — subsequent playback starts instantly without network requests.
  - The upcoming track in the queue is prefetched in the background for gapless transitions.
- **Library & Playlist Management**:
  - Quick source switching between "My Wave", "Liked Tracks", and custom user playlists.
  - Queue navigation (`queue`, `jump`), shuffle mode, and loop options (`off`, `all`, `track`).
- **Account Synchronization**: Instant `like` / `unlike` directly from the CLI, synced with your Yandex Music account.
- **Search & Quick Play**:
  - Formatted tabular search results (`search <query>`).
  - Search and immediately play the best match (`play <query>`).
  - Direct playback by track ID (`play --track <id>`).
- **JSON Output (`--json`)**: Structured machine-readable output for custom bars, scripts, and automation.

---

## Installation & Build

### Prerequisites (ALSA headers)

- **Arch Linux**:
  ```bash
  sudo pacman -S alsa-lib pkgconf base-devel
  ```
- **Ubuntu / Debian**:
  ```bash
  sudo apt install libasound2-dev pkg-config build-essential
  ```
- **Fedora**:
  ```bash
  sudo dnf install alsa-lib-devel pkgconfig gcc
  ```

### Building from source

```bash
git clone https://github.com/frosty/yamusic-cli.git
cd yamusic-cli
cargo build --release
cp target/release/yamusic-cli ~/.local/bin/
```

Or install directly via Cargo:

```bash
cargo install --path .
```

---

## Authentication (OAuth Token)

`yamusic-cli` resolves your Yandex Music token in the following order:

1. CLI flag: `--token <TOKEN>`
2. Environment variable: `YANDEX_MUSIC_TOKEN`
3. Config file: `~/.config/yamusic-cli/config.toml` (`token = "..."`)
4. yamusic-tui config: `~/.config/yamusic-tui/config.yaml` (`token: "..."`)

### How to get an OAuth token:

- **Method 1 (Browser link)**:
  1. Log into your Yandex account in any browser.
  2. Open the authorization link:
     `https://oauth.yandex.ru/authorize?response_type=token&client_id=23cabb9fd14c463bb1357da300b961c5`
  3. Click **Allow**. You will be redirected to an address like:
     ```text
     https://music.yandex.ru/#access_token=y0_AgAAAA...&token_type=bearer
     ```
  4. Copy the value of `access_token`.
- **Method 2 (Browser DevTools)**:
  1. Open [music.yandex.ru](https://music.yandex.ru) and press `F12` -> **Network** tab.
  2. Filter by `api.music.yandex.net` and inspect the `Authorization` request header:
     ```text
     Authorization: OAuth y0_AgAAAA...
     ```

---

## Configuration

Configuration is stored in `~/.config/yamusic-cli/config.toml`:

```toml
# Yandex Music OAuth token
token = "y0_AgAAAA..."

# Initial volume from 0.0 to 1.0 (default: 0.7)
volume = 0.7

# Enable caching MP3 tracks to disk (default: true)
cache_enabled = true

# Custom cache directory (optional, default: ~/.cache/yamusic-cli/tracks)
# cache_dir = "/home/frosty/.cache/yamusic-cli/tracks"

# Stream bitrate: 320 or 192 (default: 320)
bitrate = 320

# Enable MPRIS2 D-Bus server (default: true)
mpris_enabled = true

# Fade in/out duration in milliseconds for play/pause and track transitions (default: 200, 0 to disable)
fade_duration_ms = 200
```

---

## Quick Start

### 1. systemd user service (recommended)

```bash
# Install unit to ~/.config/systemd/user/yamusic-cli.service
yamusic-cli service install

# Enable and start service
systemctl --user enable --now yamusic-cli

# Check status or stream logs
yamusic-cli service status
journalctl --user -u yamusic-cli -f
```

### 2. Manual execution

```bash
# Run in background (detached)
yamusic-cli daemon --detach

# Run in foreground with logs
yamusic-cli daemon

# Stop running daemon
yamusic-cli quit
```

---

## CLI Command Reference

### Playback & Status

| Command                     | Description                                                       |
| --------------------------- | ----------------------------------------------------------------- |
| `yamusic-cli status`        | Show current track, progress bar, volume, source, and queue state |
| `yamusic-cli status --json` | Print player status as structured JSON                            |
| `yamusic-cli play`          | Resume playback                                                   |
| `yamusic-cli pause`         | Pause playback                                                    |
| `yamusic-cli toggle`        | Toggle playback / pause                                           |
| `yamusic-cli stop`          | Stop playback                                                     |
| `yamusic-cli next`          | Skip to next track                                                |
| `yamusic-cli prev`          | Skip to previous track                                            |

### Seeking & Volume

```bash
# Seeking (relative or absolute time)
yamusic-cli seek +15       # seek 15 seconds forward
yamusic-cli seek -10       # seek 10 seconds backward
yamusic-cli seek 1:30      # jump to 01:30
yamusic-cli seek 90        # jump to 90 seconds

# Volume control
yamusic-cli volume         # show current volume
yamusic-cli volume 80      # set volume to 80%
yamusic-cli volume +5      # increase volume by 5%
yamusic-cli volume -5      # decrease volume by 5%
```

### Search & Playback

```bash
# Search and display formatted table
yamusic-cli search "queen bohemian rhapsody"

# Search and immediately play the top match
yamusic-cli play "linkin park numb"
yamusic-cli play-search "daft punk get lucky"

# Play track directly by ID
yamusic-cli play --track 141109862
```

### Collection & "My Wave"

```bash
# Switch playback to "My Wave"
yamusic-cli wave

# Switch playback to Liked tracks
yamusic-cli liked

# List available user playlists
yamusic-cli playlists

# Switch to playlist by name or kind ID
yamusic-cli playlist "Favorites"
yamusic-cli playlist 1003
```

### Queue & Modes

```bash
# Display playback queue
yamusic-cli queue

# Jump to track number in queue (1-based)
yamusic-cli jump 5

# Shuffle mode
yamusic-cli shuffle        # toggle
yamusic-cli shuffle on     # enable
yamusic-cli shuffle off    # disable

# Loop mode
yamusic-cli loop           # cycle mode (off -> all -> track)
yamusic-cli loop off       # disable loop
yamusic-cli loop all       # repeat whole queue
yamusic-cli loop track     # repeat current track
```

### Favorites (Likes)

```bash
# Like currently playing track
yamusic-cli like

# Remove like from currently playing track
yamusic-cli unlike

# Like / unlike by specific track ID
yamusic-cli like 141109862
yamusic-cli unlike 141109862
```

---

## Desktop & Status Bar Integration

### Waybar

`yamusic-cli` implements the standard `org.mpris.MediaPlayer2.yamusic-cli` D-Bus interface, so it works out of the box with Waybar's built-in `mpris` module:

```jsonc
// ~/.config/waybar/config.jsonc
"mpris": {
    "player": "yamusic-cli",
    "format": "{player_icon} {artist} — {title}",
    "format-paused": "⏸ <i>{artist} — {title}</i>",
    "player-icons": {
        "default": "▶"
    },
    "max-length": 45,
    "on-click": "yamusic-cli toggle",
    "on-click-middle": "yamusic-cli like",
    "on-click-right": "yamusic-cli next",
    "on-scroll-up": "yamusic-cli volume +5",
    "on-scroll-down": "yamusic-cli volume -5"
}
```

### Media Keys (Hyprland / Sway / i3)

#### Hyprland (`hyprland.conf`):

```ini
bindl = , XF86AudioPlay,        exec, yamusic-cli toggle
bindl = , XF86AudioNext,        exec, yamusic-cli next
bindl = , XF86AudioPrev,        exec, yamusic-cli prev
bindl = , XF86AudioRaiseVolume, exec, yamusic-cli volume +5
bindl = , XF86AudioLowerVolume, exec, yamusic-cli volume -5
```

#### Sway / i3 (`config`):

```ini
bindsym XF86AudioPlay        exec yamusic-cli toggle
bindsym XF86AudioNext        exec yamusic-cli next
bindsym XF86AudioPrev        exec yamusic-cli prev
bindsym XF86AudioRaiseVolume exec yamusic-cli volume +5
bindsym XF86AudioLowerVolume exec yamusic-cli volume -5
```

### playerctl

```bash
playerctl -p yamusic-cli play-pause
playerctl -p yamusic-cli next
playerctl -p yamusic-cli previous
playerctl -p yamusic-cli metadata --format '{{ artist }} — {{ title }}'
```

---

## Project Structure

```text
src/
├── main.rs      # CLI parsing (clap) and command dispatch
├── daemon.rs    # Core daemon runtime, IPC server, prefetch worker
├── audio.rs     # Audio engine via rodio (symphonia-mp3, CPAL)
├── api.rs       # Yandex Music HTTP client (OAuth, Rotor API, CDN)
├── mpris.rs     # MPRIS2 D-Bus server (mpris-server / zbus)
├── ipc.rs       # Unix Domain Socket IPC client / server
├── queue.rs     # Playback queue, shuffle, and loop handling
├── cache.rs     # Local MP3 disk cache (~/.cache/yamusic-cli/tracks/)
├── client.rs    # CLI client execution and terminal formatting
├── config.rs    # Configuration loading (~/.config/yamusic-cli/config.toml)
└── types.rs     # Data models, IPC request / response types
```
