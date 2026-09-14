use std::sync::Arc;
use std::time::Duration;
use anyhow::Result;
use mpris_server::{
    Metadata, PlaybackRate, PlaybackStatus, PlayerInterface, Property, RootInterface,
    Server, Signal, Time, TrackId, Volume, zbus::fdo,
};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::RwLock;
use tracing::info;
use crate::audio::SharedAudioState;
use crate::queue::PlayQueue;
use crate::types::{IpcRequest, PlaybackState, Track};

fn ensure_dbus_session_bus() {
    if let Ok(addr) = std::env::var("DBUS_SESSION_BUS_ADDRESS") {
        if let Some(path_str) = addr.strip_prefix("unix:path=") {
            let path = path_str.split(',').next().unwrap_or(path_str);
            if std::path::Path::new(path).exists() {
                return;
            }
        }
    }

    let uid = std::env::var("UID").unwrap_or_else(|_| "1000".to_string());
    let run_bus = std::path::PathBuf::from(format!("/run/user/{}/bus", uid));
    if run_bus.exists() {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={}", run_bus.display()));
        return;
    }

    if let Ok(entries) = std::fs::read_dir("/tmp") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("dbus-") {
                let path = entry.path();
                if std::os::unix::net::UnixStream::connect(&path).is_ok() {
                    std::env::set_var("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={}", path.display()));
                    return;
                }
            }
        }
    }
}

pub struct MprisPlayer {
    cmd_tx: UnboundedSender<IpcRequest>,
    audio_state: Arc<SharedAudioState>,
    queue: Arc<RwLock<PlayQueue>>,
}

impl RootInterface for MprisPlayer {
    async fn identity(&self) -> fdo::Result<String> {
        Ok("Yandex Music".to_string())
    }
    async fn desktop_entry(&self) -> fdo::Result<String> {
        Ok("yamusic-cli".to_string())
    }
    async fn can_quit(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn quit(&self) -> fdo::Result<()> {
        Ok(())
    }
    async fn can_raise(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn raise(&self) -> fdo::Result<()> {
        Ok(())
    }
    async fn can_set_fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn set_fullscreen(&self, _fullscreen: bool) -> mpris_server::zbus::Result<()> {
        Ok(())
    }
    async fn has_track_list(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn supported_uri_schemes(&self) -> fdo::Result<Vec<String>> {
        Ok(vec![])
    }
    async fn supported_mime_types(&self) -> fdo::Result<Vec<String>> {
        Ok(vec![])
    }
}

impl PlayerInterface for MprisPlayer {
    async fn next(&self) -> fdo::Result<()> {
        let _ = self.cmd_tx.send(IpcRequest::Next);
        Ok(())
    }
    async fn previous(&self) -> fdo::Result<()> {
        let _ = self.cmd_tx.send(IpcRequest::Prev);
        Ok(())
    }
    async fn pause(&self) -> fdo::Result<()> {
        let _ = self.cmd_tx.send(IpcRequest::Pause);
        Ok(())
    }
    async fn play_pause(&self) -> fdo::Result<()> {
        let _ = self.cmd_tx.send(IpcRequest::Toggle);
        Ok(())
    }
    async fn stop(&self) -> fdo::Result<()> {
        let _ = self.cmd_tx.send(IpcRequest::Stop);
        Ok(())
    }
    async fn play(&self) -> fdo::Result<()> {
        let _ = self.cmd_tx.send(IpcRequest::Play { query: None, track_id: None });
        Ok(())
    }
    async fn seek(&self, offset: Time) -> fdo::Result<()> {
        let secs = offset.as_millis() as f64 / 1000.0;
        let _ = self.cmd_tx.send(IpcRequest::Seek { seconds: secs, relative: true });
        Ok(())
    }
    async fn set_position(&self, _track_id: TrackId, position: Time) -> fdo::Result<()> {
        let secs = position.as_millis() as f64 / 1000.0;
        let _ = self.cmd_tx.send(IpcRequest::Seek { seconds: secs, relative: false });
        Ok(())
    }
    async fn open_uri(&self, _uri: String) -> fdo::Result<()> {
        Ok(())
    }
    async fn playback_status(&self) -> fdo::Result<PlaybackStatus> {
        let status = match self.audio_state.state() {
            PlaybackState::Playing => PlaybackStatus::Playing,
            PlaybackState::Paused => PlaybackStatus::Paused,
            PlaybackState::Stopped => PlaybackStatus::Stopped,
        };
        Ok(status)
    }
    async fn loop_status(&self) -> fdo::Result<mpris_server::LoopStatus> {
        let q = self.queue.read().await;
        let s = match q.loop_mode() {
            crate::types::LoopMode::Off => mpris_server::LoopStatus::None,
            crate::types::LoopMode::All => mpris_server::LoopStatus::Playlist,
            crate::types::LoopMode::Track => mpris_server::LoopStatus::Track,
        };
        Ok(s)
    }
    async fn set_loop_status(&self, loop_status: mpris_server::LoopStatus) -> mpris_server::zbus::Result<()> {
        let mode = match loop_status {
            mpris_server::LoopStatus::None => crate::types::LoopMode::Off,
            mpris_server::LoopStatus::Track => crate::types::LoopMode::Track,
            mpris_server::LoopStatus::Playlist => crate::types::LoopMode::All,
        };
        let _ = self.cmd_tx.send(IpcRequest::Loop { mode: Some(mode) });
        Ok(())
    }
    async fn rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(PlaybackRate::default())
    }
    async fn set_rate(&self, _rate: PlaybackRate) -> mpris_server::zbus::Result<()> {
        Ok(())
    }
    async fn shuffle(&self) -> fdo::Result<bool> {
        let q = self.queue.read().await;
        Ok(q.is_shuffle())
    }
    async fn set_shuffle(&self, shuffle: bool) -> mpris_server::zbus::Result<()> {
        let _ = self.cmd_tx.send(IpcRequest::Shuffle { enable: Some(shuffle) });
        Ok(())
    }
    async fn metadata(&self) -> fdo::Result<Metadata> {
        let q = self.queue.read().await;
        if let Some(t) = q.current_track() {
            let artists: Vec<&str> = t.artists.iter().map(|a| a.name.as_str()).collect();
            let mut builder = Metadata::builder()
                .title(&t.title)
                .artist(artists)
                .album(t.album_title())
                .length(Time::from_millis(t.duration_ms as i64));

            if let Some(art) = t.cover_url(400) {
                builder = builder.art_url(art);
            }

            let safe_id = t.id.replace(|c: char| !c.is_alphanumeric(), "_");
            if let Ok(track_id) = TrackId::try_from(format!("/org/yamusic/track/{}", safe_id)) {
                builder = builder.trackid(track_id);
            }
            return Ok(builder.build());
        }
        Ok(Metadata::default())
    }
    async fn volume(&self) -> fdo::Result<Volume> {
        Ok(self.audio_state.volume() as f64)
    }
    async fn set_volume(&self, volume: Volume) -> mpris_server::zbus::Result<()> {
        let _ = self.cmd_tx.send(IpcRequest::Volume { value: Some(volume as f32), delta: None });
        Ok(())
    }
    async fn position(&self) -> fdo::Result<Time> {
        Ok(Time::from_millis(self.audio_state.position().as_millis() as i64))
    }
    async fn minimum_rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(PlaybackRate::default())
    }
    async fn maximum_rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(PlaybackRate::default())
    }
    async fn can_go_next(&self) -> fdo::Result<bool> {
        let q = self.queue.read().await;
        Ok(!q.is_empty())
    }
    async fn can_go_previous(&self) -> fdo::Result<bool> {
        let q = self.queue.read().await;
        Ok(!q.is_empty())
    }
    async fn can_play(&self) -> fdo::Result<bool> {
        Ok(true)
    }
    async fn can_pause(&self) -> fdo::Result<bool> {
        Ok(true)
    }
    async fn can_seek(&self) -> fdo::Result<bool> {
        Ok(true)
    }
    async fn can_control(&self) -> fdo::Result<bool> {
        Ok(true)
    }
}

pub struct MprisHandler {
    server: Server<MprisPlayer>,
}

impl MprisHandler {
    pub async fn start(
        cmd_tx: UnboundedSender<IpcRequest>,
        audio_state: Arc<SharedAudioState>,
        queue: Arc<RwLock<PlayQueue>>,
    ) -> Result<Self> {
        ensure_dbus_session_bus();
        let player = MprisPlayer {
            cmd_tx,
            audio_state,
            queue,
        };
        let server = Server::new("yamusic-cli", player).await?;
        info!("MPRIS server initialized on D-Bus as org.mpris.MediaPlayer2.yamusic-cli");
        Ok(Self { server })
    }

    pub async fn update_playback_status(&self, state: PlaybackState) {
        let status = match state {
            PlaybackState::Playing => PlaybackStatus::Playing,
            PlaybackState::Paused => PlaybackStatus::Paused,
            PlaybackState::Stopped => PlaybackStatus::Stopped,
        };
        let _ = self
            .server
            .properties_changed([Property::PlaybackStatus(status)])
            .await;
    }

    pub async fn update_track(&self, track: Option<&Track>) {
        let mut builder = Metadata::builder();
        if let Some(t) = track {
            let artists: Vec<&str> = t.artists.iter().map(|a| a.name.as_str()).collect();
            builder = builder
                .title(&t.title)
                .artist(artists)
                .album(t.album_title())
                .length(Time::from_millis(t.duration_ms as i64));

            if let Some(art) = t.cover_url(400) {
                builder = builder.art_url(art);
            }

            let safe_id = t.id.replace(|c: char| !c.is_alphanumeric(), "_");
            if let Ok(track_id) = TrackId::try_from(format!("/org/yamusic/track/{}", safe_id)) {
                builder = builder.trackid(track_id);
            }
        }
        let _ = self
            .server
            .properties_changed([Property::Metadata(builder.build())])
            .await;
    }

    pub async fn update_volume(&self, vol: f32) {
        let _ = self
            .server
            .properties_changed([Property::Volume(vol as f64)])
            .await;
    }

    pub async fn seeked(&self, pos: Duration) {
        let _ = self
            .server
            .emit(Signal::Seeked {
                position: Time::from_millis(pos.as_millis() as i64),
            })
            .await;
    }
}
