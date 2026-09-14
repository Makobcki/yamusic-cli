use std::collections::HashSet;
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use anyhow::{bail, Context, Result};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::api::YaMusicClient;
use crate::audio::{AudioEvent, AudioPlayer};
use crate::cache::TrackCache;
use crate::config::Config;
use crate::mpris::MprisHandler;
use crate::queue::{PlayQueue, QueueSource};
use crate::types::*;

pub struct Daemon {
    api: Arc<YaMusicClient>,
    user_id: u64,
    audio: AudioPlayer,
    queue: Arc<RwLock<PlayQueue>>,
    cache: TrackCache,
    liked_ids: Arc<RwLock<HashSet<String>>>,
    mpris: Option<Arc<MprisHandler>>,
    config: Config,
    #[allow(dead_code)]
    cmd_tx: UnboundedSender<IpcRequest>,
}

impl Daemon {
    pub async fn start(config: Config) -> Result<()> {
        if config.token.is_empty() {
            bail!("Yandex Music token is not configured. Provide it via --token, YANDEX_MUSIC_TOKEN env, or in ~/.config/yamusic-cli/config.toml");
        }

        info!("Starting Yandex Music client...");
        let api = Arc::new(YaMusicClient::new(config.token.clone())?);

        // Verify token and get user ID
        let status = api
            .get_status()
            .await
            .context("Failed to authenticate with Yandex Music API. Please check your token.")?;

        let user_id = status.account.uid;
        let user_login = status
            .account
            .login
            .or(status.account.full_name)
            .unwrap_or_else(|| user_id.to_string());
        info!("Authenticated as: {} (uid: {})", user_login, user_id);

        // Fetch user liked track IDs for quick lookup
        info!("Fetching liked tracks library...");
        let liked_vec = match api.get_liked_track_ids(user_id).await {
            Ok(ids) => ids,
            Err(e) => {
                warn!("Could not fetch liked tracks: {:?}. Continuing with empty likes.", e);
                Vec::new()
            }
        };
        let liked_set: HashSet<String> = liked_vec.into_iter().collect();
        info!("Loaded {} liked tracks", liked_set.len());
        let liked_ids = Arc::new(RwLock::new(liked_set));

        // Initialize Audio player
        info!("Initializing audio player with volume {}...", config.volume);
        let (audio, event_rx) = AudioPlayer::start(config.volume)?;

        let cache = TrackCache::new(config.get_cache_dir(), config.cache_enabled);
        let queue = Arc::new(RwLock::new(PlayQueue::new()));

        let (cmd_tx, mut cmd_rx) = unbounded_channel::<IpcRequest>();

        // Start MPRIS if enabled
        let mpris = if config.mpris_enabled {
            match MprisHandler::start(cmd_tx.clone(), audio.shared_state(), Arc::clone(&queue)).await {
                Ok(handler) => Some(Arc::new(handler)),
                Err(e) => {
                    warn!("MPRIS server could not be started (D-Bus unavailable?): {:?}", e);
                    None
                }
            }
        } else {
            None
        };

        let daemon = Arc::new(Self {
            api,
            user_id,
            audio,
            queue,
            cache,
            liked_ids,
            mpris,
            config,
            cmd_tx: cmd_tx.clone(),
        });

        // Background task: Listen for Audio events (track finished)
        let daemon_audio_ev = Arc::clone(&daemon);
        tokio::task::spawn_blocking(move || {
            while let Ok(event) = event_rx.recv() {
                match event {
                    AudioEvent::TrackFinished(track_id) => {
                        info!("Track {} finished playing", track_id);
                        let d = Arc::clone(&daemon_audio_ev);
                        tokio::spawn(async move {
                            d.handle_track_finished().await;
                        });
                    }
                }
            }
        });

        // Background task: Internal command processing channel (from MPRIS etc.)
        let daemon_cmd = Arc::clone(&daemon);
        tokio::spawn(async move {
            while let Some(req) = cmd_rx.recv().await {
                let _ = daemon_cmd.handle_request(req).await;
            }
        });

        // Unix Domain Socket IPC listener
        let socket_path = Config::socket_path();
        if socket_path.exists() {
            let _ = fs::remove_file(&socket_path);
        }

        let listener = UnixListener::bind(&socket_path)
            .with_context(|| format!("Failed to bind Unix socket at {}", socket_path.display()))?;

        info!("Daemon listening on Unix socket: {}", socket_path.display());

        // Default initial queue: load Liked Tracks or My Wave
        let daemon_init = Arc::clone(&daemon);
        tokio::spawn(async move {
            info!("Preloading initial music source (Моя волна)...");
            let _ = daemon_init.load_wave(false).await;
        });

        // Main IPC server accept loop
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let d = Arc::clone(&daemon);
                    tokio::spawn(async move {
                        if let Err(e) = d.handle_ipc_connection(stream).await {
                            error!("Error handling IPC client: {:?}", e);
                        }
                    });
                }
                Err(e) => {
                    error!("Error accepting IPC connection: {:?}", e);
                }
            }
        }
    }

    async fn handle_ipc_connection(&self, mut stream: tokio::net::UnixStream) -> Result<()> {
        let (reader, mut writer) = stream.split();
        let mut buf_reader = BufReader::new(reader);
        let mut line = String::new();

        while buf_reader.read_line(&mut line).await? > 0 {
            let req_str = line.trim();
            if req_str.is_empty() {
                line.clear();
                continue;
            }

            let resp = match serde_json::from_str::<IpcRequest>(req_str) {
                Ok(req) => self.handle_request(req).await,
                Err(e) => IpcResponse::err(format!("Invalid request format: {:?}", e)),
            };

            let resp_json = serde_json::to_string(&resp)?;
            writer.write_all(resp_json.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;

            if matches!(resp.data, Some(ref d) if d.get("quit").and_then(|v| v.as_bool()).unwrap_or(false)) {
                info!("Daemon received quit request. Exiting.");
                std::process::exit(0);
            }

            line.clear();
        }

        Ok(())
    }

    pub async fn handle_request(&self, req: IpcRequest) -> IpcResponse {
        match req {
            IpcRequest::Status => self.get_status().await,
            IpcRequest::Play { query, track_id } => {
                if let Some(q) = query {
                    self.play_query(&q).await
                } else if let Some(t_id) = track_id {
                    self.play_specific_track(&t_id).await
                } else {
                    self.resume_or_play_current().await
                }
            }
            IpcRequest::Pause => {
                self.audio.pause();
                if let Some(mpris) = &self.mpris {
                    mpris.update_playback_status(PlaybackState::Paused).await;
                }
                IpcResponse::ok("Paused", None)
            }
            IpcRequest::Toggle => {
                let state = self.audio.state().state();
                match state {
                    PlaybackState::Stopped => self.resume_or_play_current().await,
                    PlaybackState::Paused => {
                        self.audio.resume();
                        if let Some(mpris) = &self.mpris {
                            mpris.update_playback_status(PlaybackState::Playing).await;
                        }
                        IpcResponse::ok("Resumed", None)
                    }
                    PlaybackState::Playing => {
                        self.audio.pause();
                        if let Some(mpris) = &self.mpris {
                            mpris.update_playback_status(PlaybackState::Paused).await;
                        }
                        IpcResponse::ok("Paused", None)
                    }
                }
            }
            IpcRequest::Stop => {
                self.audio.stop();
                if let Some(mpris) = &self.mpris {
                    mpris.update_playback_status(PlaybackState::Stopped).await;
                    mpris.update_track(None).await;
                }
                IpcResponse::ok("Stopped", None)
            }
            IpcRequest::Next => self.next_track().await,
            IpcRequest::Prev => self.prev_track().await,
            IpcRequest::Seek { seconds, relative } => {
                if relative {
                    self.audio.seek_relative(seconds);
                } else {
                    self.audio.seek(Duration::from_secs_f64(seconds));
                }
                let pos = self.audio.state().position();
                if let Some(mpris) = &self.mpris {
                    mpris.seeked(pos).await;
                }
                IpcResponse::ok(format!("Seeked to {:?}", pos), None)
            }
            IpcRequest::Volume { value, delta } => {
                if let Some(v) = value {
                    self.audio.set_volume(v);
                } else if let Some(d) = delta {
                    self.audio.set_volume_relative(d);
                }
                let vol = self.audio.state().volume();
                if let Some(mpris) = &self.mpris {
                    mpris.update_volume(vol).await;
                }
                IpcResponse::ok(format!("Volume: {:.0}%", vol * 100.0), Some(json!({ "volume": vol })))
            }
            IpcRequest::Like { track_id } => self.like_track(track_id).await,
            IpcRequest::Unlike { track_id } => self.unlike_track(track_id).await,
            IpcRequest::Playlists => self.list_playlists().await,
            IpcRequest::PlayPlaylist { name_or_kind } => self.play_playlist(&name_or_kind).await,
            IpcRequest::PlayWave => self.load_wave(true).await,
            IpcRequest::PlayLiked => self.load_liked(true).await,
            IpcRequest::Search { query } => self.search_tracks(&query).await,
            IpcRequest::Queue => self.get_queue_info().await,
            IpcRequest::Jump { index } => self.jump_to_index(index).await,
            IpcRequest::Shuffle { enable } => {
                let mut q = self.queue.write().await;
                let new_val = match enable {
                    Some(e) => {
                        q.set_shuffle(e);
                        e
                    }
                    None => q.toggle_shuffle(),
                };
                IpcResponse::ok(format!("Shuffle: {}", if new_val { "on" } else { "off" }), Some(json!({ "shuffle": new_val })))
            }
            IpcRequest::Loop { mode } => {
                let mut q = self.queue.write().await;
                let new_mode = match mode {
                    Some(m) => {
                        q.set_loop_mode(m);
                        m
                    }
                    None => match q.loop_mode() {
                        LoopMode::Off => {
                            q.set_loop_mode(LoopMode::All);
                            LoopMode::All
                        }
                        LoopMode::All => {
                            q.set_loop_mode(LoopMode::Track);
                            LoopMode::Track
                        }
                        LoopMode::Track => {
                            q.set_loop_mode(LoopMode::Off);
                            LoopMode::Off
                        }
                    },
                };
                IpcResponse::ok(format!("Loop mode: {:?}", new_mode), Some(json!({ "loop": format!("{:?}", new_mode) })))
            }
            IpcRequest::QuitDaemon => {
                IpcResponse::ok("Quitting daemon", Some(json!({ "quit": true })))
            }
        }
    }

    async fn get_status(&self) -> IpcResponse {
        let state = self.audio.state().state();
        let pos = self.audio.state().position();
        let vol = self.audio.state().volume();
        let q = self.queue.read().await;

        let curr_track = q.current_track().cloned();
        let is_liked = if let Some(ref t) = curr_track {
            self.liked_ids.read().await.contains(&t.id)
        } else {
            false
        };

        let status = PlayerStatus {
            state,
            current_track: curr_track.clone(),
            position_ms: pos.as_millis() as u64,
            duration_ms: curr_track.as_ref().map(|t| t.duration_ms).unwrap_or(0),
            volume: vol,
            is_liked,
            loop_mode: q.loop_mode(),
            shuffle: q.is_shuffle(),
            source_name: q.source_name().to_string(),
            queue_index: q.current_index().unwrap_or(0),
            queue_len: q.len(),
        };

        IpcResponse::ok_data(serde_json::to_value(status).unwrap_or_default())
    }

    async fn resume_or_play_current(&self) -> IpcResponse {
        let state = self.audio.state().state();
        if state == PlaybackState::Paused {
            self.audio.resume();
            if let Some(mpris) = &self.mpris {
                mpris.update_playback_status(PlaybackState::Playing).await;
            }
            return IpcResponse::ok("Resumed playback", None);
        }

        let idx = {
            let q = self.queue.read().await;
            q.current_index().unwrap_or(0)
        };

        match self.play_index(idx).await {
            Ok(_) => IpcResponse::ok("Playback started", None),
            Err(e) => IpcResponse::err(format!("Failed to start playback: {:?}", e)),
        }
    }

    async fn play_query(&self, query: &str) -> IpcResponse {
        info!("Searching and playing query: {}", query);
        match self.api.search(query).await {
            Ok(tracks) if !tracks.is_empty() => {
                let first_title = format!("{} — {}", tracks[0].artists_str(), tracks[0].title);
                {
                    let mut q = self.queue.write().await;
                    q.set_tracks(
                        format!("Search: {}", query),
                        QueueSource::Search { query: query.to_string() },
                        tracks,
                        0,
                    );
                }
                match self.play_index(0).await {
                    Ok(_) => IpcResponse::ok(format!("Playing: {}", first_title), None),
                    Err(e) => IpcResponse::err(format!("Failed to play: {:?}", e)),
                }
            }
            Ok(_) => IpcResponse::err(format!("No tracks found for query '{}'", query)),
            Err(e) => IpcResponse::err(format!("Search failed: {:?}", e)),
        }
    }

    async fn play_specific_track(&self, track_id: &str) -> IpcResponse {
        match self.api.get_tracks(&[track_id.to_string()]).await {
            Ok(tracks) if !tracks.is_empty() => {
                let track = tracks.into_iter().next().unwrap();
                let title = format!("{} — {}", track.artists_str(), track.title);
                {
                    let mut q = self.queue.write().await;
                    q.set_tracks(
                        format!("Track: {}", title),
                        QueueSource::Custom,
                        vec![track],
                        0,
                    );
                }
                match self.play_index(0).await {
                    Ok(_) => IpcResponse::ok(format!("Playing: {}", title), None),
                    Err(e) => IpcResponse::err(format!("Failed to play: {:?}", e)),
                }
            }
            Ok(_) => IpcResponse::err(format!("Track {} not found", track_id)),
            Err(e) => IpcResponse::err(format!("Failed to fetch track: {:?}", e)),
        }
    }

    async fn play_index(&self, index: usize) -> Result<()> {
        let (track, is_wave, remaining) = {
            let mut q = self.queue.write().await;
            let track = match q.jump(index) {
                Some(t) => t.clone(),
                None => bail!("Track index {} out of range", index),
            };
            (track, q.is_my_wave(), q.tracks_remaining())
        };

        info!("Preparing to play: {} — {}", track.artists_str(), track.title);

        // Fetch audio bytes (cache or download)
        let bytes = match self.cache.get(&track.id) {
            Some(cached) => {
                info!("Track {} loaded from disk cache", track.id);
                cached
            }
            None => {
                info!("Downloading track {} from Yandex Music CDN...", track.id);
                let download_url = self
                    .api
                    .resolve_download_url(&track.id, self.config.bitrate)
                    .await
                    .context("Failed to resolve download URL")?;
                let data = self
                    .api
                    .download_track_bytes(&download_url)
                    .await
                    .context("Failed to download track bytes")?;
                let _ = self.cache.put(&track.id, &data);
                data
            }
        };

        // Play in audio sink
        self.audio.play(bytes, track.id.clone(), Duration::ZERO);

        // Update MPRIS
        if let Some(mpris) = &self.mpris {
            mpris.update_track(Some(&track)).await;
            mpris.update_playback_status(PlaybackState::Playing).await;
        }

        // Background: Prefetch next track
        let next_track_id = {
            let q = self.queue.read().await;
            q.tracks().get(index + 1).map(|t| t.id.clone())
        };
        if let Some(next_id) = next_track_id {
            let api = Arc::clone(&self.api);
            let cache = self.cache.clone();
            let bitrate = self.config.bitrate;
            tokio::spawn(async move {
                if !cache.has(&next_id) {
                    if let Ok(url) = api.resolve_download_url(&next_id, bitrate).await {
                        if let Ok(data) = api.download_track_bytes(&url).await {
                            let _ = cache.put(&next_id, &data);
                        }
                    }
                }
            });
        }

        // Background: Refill My Wave if remaining tracks low
        if is_wave && remaining < 4 {
            let d = Arc::clone(&self.api);
            let q_arc = Arc::clone(&self.queue);
            tokio::spawn(async move {
                let session_id = {
                    let q = q_arc.read().await;
                    match q.source() {
                        QueueSource::MyWave { session_id, .. } => Some(session_id.clone()),
                        _ => None,
                    }
                };
                if let Some(sess_id) = session_id {
                    let recent_keys: Vec<String> = {
                        let q = q_arc.read().await;
                        q.tracks()
                            .iter()
                            .rev()
                            .take(5)
                            .map(|t| format!("{}:{}", t.id, t.album_id().unwrap_or(0)))
                            .collect()
                    };
                    if let Ok(new_tracks) = d.rotor_next_tracks(&sess_id, &recent_keys).await {
                        info!("Auto-refilled My Wave queue with {} new tracks", new_tracks.len());
                        q_arc.write().await.append_tracks(new_tracks);
                    }
                }
            });
        }

        Ok(())
    }

    async fn handle_track_finished(&self) {
        let (next_index, has_next) = {
            let mut q = self.queue.write().await;
            match q.next() {
                Some((idx, _)) => (idx, true),
                None => (0, false),
            }
        };

        if has_next {
            if let Err(e) = self.play_index(next_index).await {
                error!("Error autoplaying next track: {:?}", e);
            }
        } else {
            info!("Reached end of playback queue");
            if let Some(mpris) = &self.mpris {
                mpris.update_playback_status(PlaybackState::Stopped).await;
            }
        }
    }

    async fn next_track(&self) -> IpcResponse {
        let (next_index, has_next) = {
            let mut q = self.queue.write().await;
            match q.next() {
                Some((idx, _)) => (idx, true),
                None => (0, false),
            }
        };

        if has_next {
            match self.play_index(next_index).await {
                Ok(_) => {
                    let q = self.queue.read().await;
                    let t = q.current_track().unwrap();
                    IpcResponse::ok(format!("Next: {} — {}", t.artists_str(), t.title), None)
                }
                Err(e) => IpcResponse::err(format!("Failed to play next: {:?}", e)),
            }
        } else {
            IpcResponse::err("End of queue")
        }
    }

    async fn prev_track(&self) -> IpcResponse {
        let (prev_index, has_prev) = {
            let mut q = self.queue.write().await;
            match q.previous() {
                Some((idx, _)) => (idx, true),
                None => (0, false),
            }
        };

        if has_prev {
            match self.play_index(prev_index).await {
                Ok(_) => {
                    let q = self.queue.read().await;
                    let t = q.current_track().unwrap();
                    IpcResponse::ok(format!("Previous: {} — {}", t.artists_str(), t.title), None)
                }
                Err(e) => IpcResponse::err(format!("Failed to play previous: {:?}", e)),
            }
        } else {
            IpcResponse::err("Beginning of queue")
        }
    }

    async fn jump_to_index(&self, index: usize) -> IpcResponse {
        match self.play_index(index).await {
            Ok(_) => {
                let q = self.queue.read().await;
                let t = q.current_track().unwrap();
                IpcResponse::ok(format!("Jumped to [{}]: {} — {}", index, t.artists_str(), t.title), None)
            }
            Err(e) => IpcResponse::err(format!("Failed to jump to index {}: {:?}", index, e)),
        }
    }

    async fn like_track(&self, track_id: Option<String>) -> IpcResponse {
        let id = match track_id {
            Some(i) => i,
            None => {
                let q = self.queue.read().await;
                match q.current_track() {
                    Some(t) => t.id.clone(),
                    None => return IpcResponse::err("No track currently playing"),
                }
            }
        };

        match self.api.like_track(self.user_id, &id).await {
            Ok(_) => {
                self.liked_ids.write().await.insert(id.clone());
                IpcResponse::ok(format!("Liked track {}", id), Some(json!({ "liked": true, "track_id": id })))
            }
            Err(e) => IpcResponse::err(format!("Failed to like track: {:?}", e)),
        }
    }

    async fn unlike_track(&self, track_id: Option<String>) -> IpcResponse {
        let id = match track_id {
            Some(i) => i,
            None => {
                let q = self.queue.read().await;
                match q.current_track() {
                    Some(t) => t.id.clone(),
                    None => return IpcResponse::err("No track currently playing"),
                }
            }
        };

        match self.api.unlike_track(self.user_id, &id).await {
            Ok(_) => {
                self.liked_ids.write().await.remove(&id);
                IpcResponse::ok(format!("Unliked track {}", id), Some(json!({ "liked": false, "track_id": id })))
            }
            Err(e) => IpcResponse::err(format!("Failed to unlike track: {:?}", e)),
        }
    }

    pub async fn load_wave(&self, start_playback: bool) -> IpcResponse {
        info!("Initializing My Wave session...");
        match self.api.rotor_new_session().await {
            Ok(session) => {
                let session_id = session.radio_session_id.unwrap_or_default();
                let batch_id = session.batch_id;
                let tracks: Vec<Track> = session.sequence.into_iter().map(|it| it.track).collect();

                if tracks.is_empty() {
                    return IpcResponse::err("My Wave returned no tracks");
                }

                info!("My Wave started: {} tracks loaded", tracks.len());
                {
                    let mut q = self.queue.write().await;
                    q.set_tracks(
                        "Моя волна".to_string(),
                        QueueSource::MyWave { session_id, batch_id },
                        tracks,
                        0,
                    );
                }

                if start_playback {
                    let _ = self.play_index(0).await;
                }

                IpcResponse::ok("My Wave loaded", None)
            }
            Err(e) => IpcResponse::err(format!("Failed to start My Wave: {:?}", e)),
        }
    }

    pub async fn load_liked(&self, start_playback: bool) -> IpcResponse {
        info!("Loading Liked Tracks...");
        match self.api.get_liked_track_ids(self.user_id).await {
            Ok(ids) => {
                if ids.is_empty() {
                    return IpcResponse::err("No liked tracks found");
                }
                match self.api.get_tracks(&ids).await {
                    Ok(tracks) => {
                        info!("Loaded {} liked tracks", tracks.len());
                        {
                            let mut q = self.queue.write().await;
                            q.set_tracks(
                                "Любимые треки".to_string(),
                                QueueSource::Liked,
                                tracks,
                                0,
                            );
                        }
                        if start_playback {
                            let _ = self.play_index(0).await;
                        }
                        IpcResponse::ok("Liked tracks loaded", None)
                    }
                    Err(e) => IpcResponse::err(format!("Failed to fetch track details: {:?}", e)),
                }
            }
            Err(e) => IpcResponse::err(format!("Failed to fetch liked track IDs: {:?}", e)),
        }
    }

    async fn list_playlists(&self) -> IpcResponse {
        match self.api.get_playlists(self.user_id).await {
            Ok(playlists) => {
                let mut list = Vec::new();
                list.push(json!({
                    "name": "Моя волна (My Wave)",
                    "kind": "wave",
                    "tracks": "∞",
                }));
                list.push(json!({
                    "name": "Любимые треки (Liked Tracks)",
                    "kind": "liked",
                    "tracks": self.liked_ids.read().await.len(),
                }));
                for pl in playlists {
                    list.push(json!({
                        "name": pl.title,
                        "kind": pl.kind.to_string(),
                        "tracks": pl.track_count,
                    }));
                }
                IpcResponse::ok_data(json!(list))
            }
            Err(e) => IpcResponse::err(format!("Failed to list playlists: {:?}", e)),
        }
    }

    async fn play_playlist(&self, name_or_kind: &str) -> IpcResponse {
        let lower = name_or_kind.to_lowercase();
        if lower == "wave" || lower == "моя волна" || lower == "my wave" {
            return self.load_wave(true).await;
        }
        if lower == "liked" || lower == "любимые треки" || lower == "избранное" || lower == "favorites" {
            return self.load_liked(true).await;
        }

        // Match playlist by kind or name
        match self.api.get_playlists(self.user_id).await {
            Ok(playlists) => {
                let found = playlists.into_iter().find(|pl| {
                    pl.kind.to_string() == name_or_kind
                        || pl.title.to_lowercase() == lower
                        || pl.title.to_lowercase().contains(&lower)
                });

                if let Some(pl) = found {
                    match self.api.get_playlist_tracks(self.user_id, pl.kind).await {
                        Ok(tracks) if !tracks.is_empty() => {
                            let title = pl.title.clone();
                            {
                                let mut q = self.queue.write().await;
                                q.set_tracks(
                                    format!("Playlist: {}", title),
                                    QueueSource::Playlist {
                                        kind: pl.kind,
                                        uid: self.user_id,
                                        title,
                                    },
                                    tracks,
                                    0,
                                );
                            }
                            match self.play_index(0).await {
                                Ok(_) => IpcResponse::ok(format!("Playing playlist '{}'", pl.title), None),
                                Err(e) => IpcResponse::err(format!("Failed to play: {:?}", e)),
                            }
                        }
                        Ok(_) => IpcResponse::err(format!("Playlist '{}' is empty", pl.title)),
                        Err(e) => IpcResponse::err(format!("Failed to fetch tracks: {:?}", e)),
                    }
                } else {
                    IpcResponse::err(format!("Playlist '{}' not found", name_or_kind))
                }
            }
            Err(e) => IpcResponse::err(format!("Failed to search playlists: {:?}", e)),
        }
    }

    async fn search_tracks(&self, query: &str) -> IpcResponse {
        match self.api.search(query).await {
            Ok(tracks) => {
                let items: Vec<_> = tracks
                    .iter()
                    .take(25)
                    .map(|t| {
                        json!({
                            "id": t.id,
                            "title": t.title,
                            "artists": t.artists_str(),
                            "album": t.album_title(),
                            "duration_ms": t.duration_ms,
                        })
                    })
                    .collect();
                IpcResponse::ok_data(json!(items))
            }
            Err(e) => IpcResponse::err(format!("Search error: {:?}", e)),
        }
    }

    async fn get_queue_info(&self) -> IpcResponse {
        let q = self.queue.read().await;
        let curr = q.current_index();
        let items: Vec<_> = q
            .tracks()
            .iter()
            .enumerate()
            .map(|(idx, t)| {
                json!({
                    "index": idx,
                    "is_current": Some(idx) == curr,
                    "id": t.id,
                    "title": t.title,
                    "artists": t.artists_str(),
                    "duration_ms": t.duration_ms,
                })
            })
            .collect();

        IpcResponse::ok_data(json!({
            "source": q.source_name(),
            "current_index": curr,
            "total": q.len(),
            "tracks": items,
        }))
    }
}
