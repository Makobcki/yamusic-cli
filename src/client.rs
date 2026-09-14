use colored::Colorize;
use anyhow::Result;
use serde_json::Value;

use crate::config::Config;
use crate::ipc::IpcClient;
use crate::types::{IpcRequest, PlaybackState, PlayerStatus};

pub struct Client {
    ipc: IpcClient,
}

impl Client {
    pub fn new() -> Self {
        Self {
            ipc: IpcClient::new(Config::socket_path()),
        }
    }

    pub fn is_daemon_running(&self) -> bool {
        self.ipc.is_running()
    }

    pub async fn execute(&self, req: IpcRequest, json_output: bool) -> Result<()> {
        let resp = self.ipc.send(&req).await?;

        if json_output {
            if let Some(ref data) = resp.data {
                println!("{}", serde_json::to_string_pretty(data)?);
            } else {
                println!("{}", serde_json::to_string_pretty(&resp)?);
            }
            return Ok(());
        }

        if !resp.success {
            eprintln!("{} {}", "Error:".red().bold(), resp.message.unwrap_or_default());
            return Ok(());
        }

        match req {
            IpcRequest::Status => {
                if let Some(data) = resp.data {
                    if let Ok(status) = serde_json::from_value::<PlayerStatus>(data) {
                        Self::render_status(&status);
                    }
                }
            }
            IpcRequest::Search { .. } => {
                if let Some(Value::Array(items)) = resp.data {
                    Self::render_search_results(&items);
                }
            }
            IpcRequest::Playlists => {
                if let Some(Value::Array(items)) = resp.data {
                    Self::render_playlists(&items);
                }
            }
            IpcRequest::Queue => {
                if let Some(data) = resp.data {
                    Self::render_queue(&data);
                }
            }
            _ => {
                if let Some(msg) = resp.message {
                    println!("{}", msg.green());
                }
            }
        }

        Ok(())
    }

    fn render_status(status: &PlayerStatus) {
        let state_str = match status.state {
            PlaybackState::Playing => "▶ Playing".green().bold(),
            PlaybackState::Paused => "⏸ Paused".yellow().bold(),
            PlaybackState::Stopped => "⏹ Stopped".red().bold(),
        };

        if let Some(ref t) = status.current_track {
            println!("{} {} — {}", state_str, t.artists_str().bold(), t.title.cyan().bold());
            println!("  {}:   {}", "Album".dimmed(), t.album_title());

            let pos_sec = status.position_ms / 1000;
            let dur_sec = status.duration_ms / 1000;
            let bar = render_progress_bar(status.position_ms, status.duration_ms, 25);
            println!(
                "  {}:    {:02}:{:02} / {:02}:{:02} [{}]",
                "Time".dimmed(),
                pos_sec / 60,
                pos_sec % 60,
                dur_sec / 60,
                dur_sec % 60,
                bar.cyan()
            );

            let like_str = if status.is_liked {
                "♥ Liked".red().bold()
            } else {
                "♡ Not liked".dimmed()
            };

            let loop_str = match status.loop_mode {
                crate::types::LoopMode::Off => "Off".dimmed(),
                crate::types::LoopMode::All => "All".green(),
                crate::types::LoopMode::Track => "Track".yellow(),
            };

            let shuffle_str = if status.shuffle {
                "On".green()
            } else {
                "Off".dimmed()
            };

            println!(
                "  {}:  {} | Vol: {:.0}% | Loop: {} | Shuffle: {}",
                "Status".dimmed(),
                like_str,
                status.volume * 100.0,
                loop_str,
                shuffle_str
            );

            println!(
                "  {}:  {} [{}/{}]",
                "Source".dimmed(),
                status.source_name.magenta(),
                status.queue_index + 1,
                status.queue_len
            );
        } else {
            println!("{} (No track playing)", state_str);
            println!(
                "  {}:  Vol: {:.0}% | Loop: {:?} | Shuffle: {}",
                "Status".dimmed(),
                status.volume * 100.0,
                status.loop_mode,
                if status.shuffle { "On" } else { "Off" }
            );
        }
    }

    fn render_search_results(items: &[Value]) {
        if items.is_empty() {
            println!("{}", "No tracks found.".dimmed());
            return;
        }

        println!("{}", "Search Results:".bold());
        println!("{:<3} | {:<30} | {:<25} | {:<8} | {}", "#".dimmed(), "Title", "Artist", "Time", "ID".dimmed());
        println!("{:-<3}-+-{:-<30}-+-{:-<25}-+-{:-<8}-+-{:-<12}", "", "", "", "", "");

        for (idx, item) in items.iter().enumerate() {
            let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let title = truncate(item.get("title").and_then(|v| v.as_str()).unwrap_or(""), 28);
            let artists = truncate(item.get("artists").and_then(|v| v.as_str()).unwrap_or(""), 23);
            let dur_ms = item.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0);
            let sec = dur_ms / 1000;

            println!(
                "{:<3} | {:<30} | {:<25} | {:02}:{:02}    | {}",
                (idx + 1).to_string().cyan(),
                title.bold(),
                artists,
                sec / 60,
                sec % 60,
                id.dimmed()
            );
        }
        println!();
        println!("{}", "Tip: Run 'yamusic-cli play-search <query>' to play directly, or 'yamusic-cli play --track <id>' to play by ID.".dimmed());
    }

    fn render_playlists(items: &[Value]) {
        println!("{}", "Available Playlists:".bold());
        println!("{:<15} | {:<35} | {}", "Kind / ID".dimmed(), "Title", "Tracks");
        println!("{:-<15}-+-{:-<35}-+-{:-<10}", "", "", "");

        for item in items {
            let kind = item.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let tracks = match item.get("tracks") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Number(n)) => n.to_string(),
                _ => "-".to_string(),
            };

            println!(
                "{:<15} | {:<35} | {}",
                kind.cyan(),
                name.bold(),
                tracks
            );
        }
        println!();
        println!("{}", "Tip: Run 'yamusic-cli playlist <name or kind>' to switch playlist.".dimmed());
    }

    fn render_queue(data: &Value) {
        let source = data.get("source").and_then(|v| v.as_str()).unwrap_or("Queue");
        let total = data.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
        let curr_idx = data.get("current_index").and_then(|v| v.as_u64()).map(|v| v as usize);

        println!("{} ({} tracks):", source.magenta().bold(), total);

        if let Some(Value::Array(tracks)) = data.get("tracks") {
            for track in tracks {
                let idx = track.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let is_curr = curr_idx == Some(idx);
                let title = truncate(track.get("title").and_then(|v| v.as_str()).unwrap_or(""), 35);
                let artists = truncate(track.get("artists").and_then(|v| v.as_str()).unwrap_or(""), 25);
                let dur_ms = track.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0);
                let sec = dur_ms / 1000;

                let marker = if is_curr {
                    "▶".green().bold()
                } else {
                    " ".normal()
                };

                println!(
                    "{} {:>3}. {:<35} — {:<25} {:02}:{:02}",
                    marker,
                    idx + 1,
                    if is_curr { title.cyan().bold() } else { title.normal() },
                    artists.dimmed(),
                    sec / 60,
                    sec % 60
                );
            }
        }
    }
}

fn render_progress_bar(pos_ms: u64, dur_ms: u64, width: usize) -> String {
    if dur_ms == 0 {
        return "─".repeat(width);
    }
    let progress = (pos_ms as f64 / dur_ms as f64).clamp(0.0, 1.0);
    let filled = (progress * width as f64).round() as usize;
    let filled = filled.min(width);

    let mut bar = String::with_capacity(width * 4);
    for i in 0..width {
        if i == filled {
            bar.push('●');
        } else if i < filled {
            bar.push('━');
        } else {
            bar.push('─');
        }
    }
    bar
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() > max_len {
        let truncated: String = s.chars().take(max_len - 1).collect();
        format!("{}…", truncated)
    } else {
        s.to_string()
    }
}
