use std::fs;
use std::path::PathBuf;
use std::process::Command;
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;
use tracing_subscriber::EnvFilter;

use yamusic_cli::client::Client;
use yamusic_cli::config::Config;
use yamusic_cli::daemon::Daemon;
use yamusic_cli::types::{IpcRequest, LoopMode};


#[derive(Parser)]
#[command(
    name = "yamusic-cli",
    author = "frosty",
    version = "0.1.0",
    about = "Lightweight Yandex Music daemon & CLI client without GUI and tracking"
)]
struct Cli {
    #[arg(long, global = true, help = "Output raw JSON instead of formatted text")]
    json: bool,

    #[arg(long, global = true, help = "Override Yandex Music OAuth token")]
    token: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "Start yamusic-cli daemon service")]
    Daemon {
        #[arg(short, long, help = "Run daemon in background (detached process)")]
        detach: bool,

        #[arg(long, help = "Fade in/out duration in milliseconds (default: 200, 0 to disable)")]
        fade_duration_ms: Option<u64>,
    },

    #[command(about = "Show current playback status")]
    Status,

    #[command(about = "Resume playback, play search query or track ID")]
    Play {
        #[arg(help = "Search query to play immediately")]
        query: Option<String>,

        #[arg(long, help = "Play specific track by Yandex Music ID")]
        track: Option<String>,
    },

    #[command(about = "Pause playback")]
    Pause,

    #[command(about = "Toggle play/pause")]
    Toggle,

    #[command(about = "Skip to next track")]
    Next,

    #[command(about = "Go to previous track")]
    Prev,

    #[command(about = "Stop playback")]
    Stop,

    #[command(about = "Seek within current track (+10, -10, 1:30, or 90)")]
    Seek {
        #[arg(help = "Seek offset (+10, -10) or absolute position (1:30, 90)")]
        time: String,
    },

    #[command(about = "Get or set volume (e.g. 80, +5, -5)")]
    Volume {
        #[arg(help = "Target volume (0-100) or delta (+5, -5)")]
        value: Option<String>,
    },

    #[command(about = "Add track to favorites (like)")]
    Like {
        #[arg(help = "Optional track ID (defaults to currently playing track)")]
        track_id: Option<String>,
    },

    #[command(about = "Remove track from favorites (unlike)")]
    Unlike {
        #[arg(help = "Optional track ID (defaults to currently playing track)")]
        track_id: Option<String>,
    },

    #[command(about = "Search tracks")]
    Search {
        #[arg(help = "Search query")]
        query: String,
    },

    #[command(about = "Search and immediately start playing the best match")]
    PlaySearch {
        #[arg(help = "Search query")]
        query: String,
    },

    #[command(about = "List available playlists")]
    Playlists,

    #[command(about = "Switch to playlist by name or kind ID")]
    Playlist {
        #[arg(help = "Playlist name or kind ID")]
        name: String,

        #[arg(short, long, help = "Shuffle tracks")]
        shuffle: bool,
    },

    #[command(about = "Switch playback to 'Моя волна' (My Wave)")]
    Wave,

    #[command(about = "Switch playback to 'Любимые треки' (Liked Tracks)")]
    Liked {
        #[arg(short, long, help = "Shuffle tracks")]
        shuffle: bool,
    },

    #[command(about = "Show current playback queue")]
    Queue,

    #[command(about = "Jump to specific index in queue")]
    Jump {
        #[arg(help = "Track index (1-based or 0-based)")]
        index: usize,
    },

    #[command(about = "Toggle or set shuffle mode")]
    Shuffle {
        #[arg(help = "on / off")]
        state: Option<String>,
    },

    #[command(about = "Set or cycle loop mode (off, all, track)")]
    Loop {
        #[arg(help = "off, all, track")]
        mode: Option<String>,
    },

    #[command(about = "Stop and exit the daemon process")]
    Quit,

    #[command(about = "Manage systemd user service for yamusic-cli")]
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
}

#[derive(Subcommand)]
enum ServiceAction {
    #[command(about = "Install systemd user service file (~/.config/systemd/user/yamusic-cli.service)")]
    Install,
    #[command(about = "Uninstall systemd user service file")]
    Uninstall,
    #[command(about = "Check systemd service status")]
    Status,
}

fn main() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    runtime.block_on(async_main())
}

async fn async_main() -> Result<()> {
    let cli = Cli::parse();

    // Default to Status if no subcommand given
    let command = cli.command.unwrap_or(Commands::Status);

    match command {
        Commands::Daemon { detach, fade_duration_ms } => {
            if detach {
                run_daemon_detached(fade_duration_ms, cli.token.as_deref())?;
                return Ok(());
            }

            // Foreground daemon
            tracing_subscriber::fmt()
                .with_env_filter(
                    EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| EnvFilter::new("info,yamusic_cli=debug,yamusic=debug")),
                )
                .init();

            let mut config = Config::load(cli.token.as_deref())?;
            if let Some(fade_ms) = fade_duration_ms {
                config.fade_duration_ms = fade_ms;
            }
            Daemon::start(config).await?;
        }
        Commands::Service { action } => match action {
            ServiceAction::Install => install_service()?,
            ServiceAction::Uninstall => uninstall_service()?,
            ServiceAction::Status => check_service_status()?,
        },
        other => {
            let client = Client::new();

            // Check if daemon is running
            if !client.is_daemon_running() {
                if cli.json {
                    println!("{}", serde_json::json!({ "status": "stopped", "running": false }));
                } else {
                    eprintln!(
                        "{} yamusic-cli daemon is not running.",
                        "Notice:".yellow().bold()
                    );
                    eprintln!(
                        "Run '{}' or '{}' to start it.",
                        "yamusic-cli daemon --detach".cyan().bold(),
                        "systemctl --user start yamusic-cli".cyan()
                    );
                }
                std::process::exit(1);
            }

            let req = match other {
                Commands::Status => IpcRequest::Status,
                Commands::Play { query, track } => IpcRequest::Play {
                    query,
                    track_id: track,
                },
                Commands::Pause => IpcRequest::Pause,
                Commands::Toggle => IpcRequest::Toggle,
                Commands::Next => IpcRequest::Next,
                Commands::Prev => IpcRequest::Prev,
                Commands::Stop => IpcRequest::Stop,
                Commands::Seek { time } => {
                    let (secs, rel) = parse_seek(&time)?;
                    IpcRequest::Seek {
                        seconds: secs,
                        relative: rel,
                    }
                }
                Commands::Volume { value } => match value {
                    Some(v) => {
                        let (abs, rel) = parse_volume(&v)?;
                        IpcRequest::Volume {
                            value: abs,
                            delta: rel,
                        }
                    }
                    None => IpcRequest::Volume {
                        value: None,
                        delta: None,
                    },
                },
                Commands::Like { track_id } => IpcRequest::Like { track_id },
                Commands::Unlike { track_id } => IpcRequest::Unlike { track_id },
                Commands::Search { query } => IpcRequest::Search { query },
                Commands::PlaySearch { query } => IpcRequest::Play {
                    query: Some(query),
                    track_id: None,
                },
                Commands::Playlists => IpcRequest::Playlists,
                Commands::Playlist { name, shuffle } => IpcRequest::PlayPlaylist {
                    name_or_kind: name,
                    shuffle: if shuffle { Some(true) } else { None },
                },
                Commands::Wave => IpcRequest::PlayWave,
                Commands::Liked { shuffle } => IpcRequest::PlayLiked {
                    shuffle: if shuffle { Some(true) } else { None },
                },
                Commands::Queue => IpcRequest::Queue,
                Commands::Jump { index } => {
                    // Convert 1-based index to 0-based if > 0
                    let idx = if index > 0 { index - 1 } else { 0 };
                    IpcRequest::Jump { index: idx }
                }
                Commands::Shuffle { state } => {
                    let enable = state.as_deref().map(|s| s == "on" || s == "true" || s == "1");
                    IpcRequest::Shuffle { enable }
                }
                Commands::Loop { mode } => {
                    let loop_mode = mode.as_deref().and_then(|m| match m.to_lowercase().as_str() {
                        "off" | "none" => Some(LoopMode::Off),
                        "all" | "queue" => Some(LoopMode::All),
                        "track" | "one" | "single" => Some(LoopMode::Track),
                        _ => None,
                    });
                    IpcRequest::Loop { mode: loop_mode }
                }
                Commands::Quit => IpcRequest::QuitDaemon,
                _ => unreachable!(),
            };

            client.execute(req, cli.json).await?;
        }
    }

    Ok(())
}

fn parse_seek(input: &str) -> Result<(f64, bool)> {
    let trimmed = input.trim();
    if trimmed.starts_with('+') || trimmed.starts_with('-') {
        let secs: f64 = trimmed.parse()?;
        Ok((secs, true))
    } else if trimmed.contains(':') {
        let parts: Vec<&str> = trimmed.split(':').collect();
        if parts.len() == 2 {
            let min: f64 = parts[0].parse()?;
            let sec: f64 = parts[1].parse()?;
            Ok((min * 60.0 + sec, false))
        } else {
            bail!("Invalid time format: {}", input);
        }
    } else {
        let secs: f64 = trimmed.parse()?;
        Ok((secs, false))
    }
}

fn parse_volume(input: &str) -> Result<(Option<f32>, Option<f32>)> {
    let trimmed = input.trim();
    if trimmed.starts_with('+') || trimmed.starts_with('-') {
        let delta: f32 = trimmed.parse()?;
        // Treat as percentage if > 1 or < -1
        let normalized = if delta.abs() > 1.0 { delta / 100.0 } else { delta };
        Ok((None, Some(normalized)))
    } else {
        let val: f32 = trimmed.parse()?;
        let normalized = if val > 1.0 { val / 100.0 } else { val };
        Ok((Some(normalized), None))
    }
}

fn run_daemon_detached(fade_duration_ms: Option<u64>, token: Option<&str>) -> Result<()> {
    #[cfg(unix)]
    use std::os::unix::process::CommandExt;

    let current_exe = std::env::current_exe()?;
    let mut cmd = Command::new(&current_exe);
    cmd.arg("daemon");
    if let Some(fade_ms) = fade_duration_ms {
        cmd.arg("--fade-duration-ms").arg(fade_ms.to_string());
    }
    if let Some(t) = token {
        cmd.arg("--token").arg(t);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    #[cfg(unix)]
    cmd.process_group(0);

    let child = cmd.spawn()?;

    println!(
        "{} Started yamusic-cli daemon in background (PID: {})",
        "✔".green().bold(),
        child.id()
    );
    Ok(())
}

fn get_systemd_user_dir() -> Result<PathBuf> {
    let config_dir = dirs::config_dir().context("Could not find user config dir")?;
    let systemd_dir = config_dir.join("systemd").join("user");
    fs::create_dir_all(&systemd_dir)?;
    Ok(systemd_dir)
}

fn install_service() -> Result<()> {
    let exe_path = std::env::current_exe()?;
    let service_dir = get_systemd_user_dir()?;
    let service_path = service_dir.join("yamusic-cli.service");

    let unit_content = format!(
        r#"[Unit]
Description=Yandex Music Daemon (yamusic-cli)
After=sound.target network-online.target
Wants=network-online.target

[Service]
Type=simple
Environment="PIPEWIRE_NODE=YandexMusic_EQ" "PULSE_SINK=YandexMusic_EQ"
ExecStart={} daemon
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
"#,
        exe_path.display()
    );

    fs::write(&service_path, unit_content)?;
    println!(
        "{} Installed systemd service to {}",
        "✔".green().bold(),
        service_path.display()
    );
    println!();
    println!("To enable and start yamusic-cli automatically:");
    println!("  {}", "systemctl --user daemon-reload".cyan());
    println!("  {}", "systemctl --user enable --now yamusic-cli".cyan());
    Ok(())
}

fn uninstall_service() -> Result<()> {
    let service_dir = get_systemd_user_dir()?;
    let service_path = service_dir.join("yamusic-cli.service");
    if service_path.exists() {
        let _ = Command::new("systemctl")
            .args(["--user", "stop", "yamusic-cli"])
            .status();
        let _ = Command::new("systemctl")
            .args(["--user", "disable", "yamusic-cli"])
            .status();
        fs::remove_file(&service_path)?;
        let _ = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status();
        println!("{} Removed yamusic-cli.service", "✔".green().bold());
    } else {
        println!("Service file not found at {}", service_path.display());
    }
    Ok(())
}

fn check_service_status() -> Result<()> {
    let status = Command::new("systemctl")
        .args(["--user", "status", "yamusic-cli"])
        .status();
    match status {
        Ok(_) => Ok(()),
        Err(e) => bail!("Failed to execute systemctl: {:?}", e),
    }
}
