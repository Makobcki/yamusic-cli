use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;
use anyhow::{Context, Result};
use rodio::{Decoder, OutputStream, Sink};
use tracing::{error, info};
use crate::types::PlaybackState;

pub enum AudioCommand {
    PlayFile {
        path: PathBuf,
        track_id: String,
        start_pos: Duration,
    },
    Pause,
    Resume,
    Toggle,
    Stop,
    Seek(Duration),
    SeekRelative(f64),
    SetVolume(f32),
    SetVolumeRelative(f32),
}

pub enum AudioEvent {
    TrackFinished(String),
}

pub struct SharedAudioState {
    pub playback_state: AtomicU8, // 0 = Stopped, 1 = Playing, 2 = Paused
    pub position_ms: AtomicU64,
    pub volume_percent: AtomicU8, // 0 - 150%
    pub current_track_id: RwLock<Option<String>>,
}

impl SharedAudioState {
    pub fn new(volume: f32) -> Self {
        Self {
            playback_state: AtomicU8::new(0),
            position_ms: AtomicU64::new(0),
            volume_percent: AtomicU8::new((volume * 100.0).clamp(0.0, 150.0) as u8),
            current_track_id: RwLock::new(None),
        }
    }

    pub fn state(&self) -> PlaybackState {
        match self.playback_state.load(Ordering::Relaxed) {
            1 => PlaybackState::Playing,
            2 => PlaybackState::Paused,
            _ => PlaybackState::Stopped,
        }
    }

    pub fn position(&self) -> Duration {
        Duration::from_millis(self.position_ms.load(Ordering::Relaxed))
    }

    pub fn volume(&self) -> f32 {
        self.volume_percent.load(Ordering::Relaxed) as f32 / 100.0
    }

    pub fn current_track_id(&self) -> Option<String> {
        self.current_track_id.read().ok().and_then(|t| t.clone())
    }
}

#[derive(Clone)]
pub struct AudioPlayer {
    cmd_tx: Sender<AudioCommand>,
    state: Arc<SharedAudioState>,
}

impl AudioPlayer {
    pub fn start(initial_volume: f32) -> Result<(Self, Receiver<AudioEvent>)> {
        let (cmd_tx, cmd_rx) = channel::<AudioCommand>();
        let (event_tx, event_rx) = channel::<AudioEvent>();
        let state = Arc::new(SharedAudioState::new(initial_volume));
        let state_clone = Arc::clone(&state);

        thread::Builder::new()
            .name("yamusic-audio".to_string())
            .spawn(move || {
                audio_thread_main(cmd_rx, event_tx, state_clone, initial_volume);
            })
            .context("Failed to spawn audio thread")?;

        Ok((
            Self { cmd_tx, state },
            event_rx,
        ))
    }

    pub fn state(&self) -> &SharedAudioState {
        &self.state
    }

    pub fn shared_state(&self) -> Arc<SharedAudioState> {
        Arc::clone(&self.state)
    }

    pub fn play_file(&self, path: PathBuf, track_id: String, start_pos: Duration) {
        let _ = self.cmd_tx.send(AudioCommand::PlayFile { path, track_id, start_pos });
    }

    pub fn pause(&self) {
        let _ = self.cmd_tx.send(AudioCommand::Pause);
    }

    pub fn resume(&self) {
        let _ = self.cmd_tx.send(AudioCommand::Resume);
    }

    pub fn toggle(&self) {
        let _ = self.cmd_tx.send(AudioCommand::Toggle);
    }

    pub fn stop(&self) {
        let _ = self.cmd_tx.send(AudioCommand::Stop);
    }

    pub fn seek(&self, pos: Duration) {
        let _ = self.cmd_tx.send(AudioCommand::Seek(pos));
    }

    pub fn seek_relative(&self, delta: f64) {
        let _ = self.cmd_tx.send(AudioCommand::SeekRelative(delta));
    }

    pub fn set_volume(&self, vol: f32) {
        let _ = self.cmd_tx.send(AudioCommand::SetVolume(vol));
    }

    pub fn set_volume_relative(&self, delta: f32) {
        let _ = self.cmd_tx.send(AudioCommand::SetVolumeRelative(delta));
    }
}

fn audio_thread_main(
    cmd_rx: Receiver<AudioCommand>,
    event_tx: Sender<AudioEvent>,
    state: Arc<SharedAudioState>,
    initial_volume: f32,
) {
    let (_stream, stream_handle) = match OutputStream::try_default() {
        Ok(res) => res,
        Err(e) => {
            error!("Failed to open audio output stream: {:?}", e);
            return;
        }
    };

    let mut sink = match Sink::try_new(&stream_handle) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to create audio sink: {:?}", e);
            return;
        }
    };

    let mut current_volume = initial_volume;
    sink.set_volume(current_volume);

    let mut current_track_id: Option<String> = None;
    let mut was_playing = false;

    loop {
        // Poll for commands with 50ms timeout for smooth position tracking
        match cmd_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(cmd) => match cmd {
                AudioCommand::PlayFile { path, track_id, start_pos } => {
                    sink = match Sink::try_new(&stream_handle) {
                        Ok(s) => s,
                        Err(e) => {
                            error!("Failed to recreate audio sink: {:?}", e);
                            continue;
                        }
                    };
                    sink.set_volume(current_volume);

                    match std::fs::File::open(&path) {
                        Ok(file) => {
                            let reader = std::io::BufReader::with_capacity(512 * 1024, file);
                            match Decoder::new(reader) {
                                Ok(decoder) => {
                                    sink.append(decoder);
                                    if start_pos > Duration::ZERO {
                                        let _ = sink.try_seek(start_pos);
                                    }
                                    sink.play();
                                    current_track_id = Some(track_id.clone());
                                    if let Ok(mut lock) = state.current_track_id.write() {
                                        *lock = Some(track_id);
                                    }
                                    state.playback_state.store(1, Ordering::Relaxed);
                                    was_playing = true;
                                }
                                Err(e) => {
                                    error!("Failed to decode audio file {}: {:?}", path.display(), e);
                                    current_track_id = None;
                                    if let Ok(mut lock) = state.current_track_id.write() {
                                        *lock = None;
                                    }
                                    state.playback_state.store(0, Ordering::Relaxed);
                                    was_playing = false;
                                }
                            }
                        }
                        Err(e) => {
                            error!("Failed to open audio file {}: {:?}", path.display(), e);
                        }
                    }
                }
                AudioCommand::Pause => {
                    sink.pause();
                    if was_playing {
                        state.playback_state.store(2, Ordering::Relaxed);
                    }
                }
                AudioCommand::Resume => {
                    if !sink.empty() {
                        sink.play();
                        state.playback_state.store(1, Ordering::Relaxed);
                    }
                }
                AudioCommand::Toggle => {
                    if sink.is_paused() {
                        sink.play();
                        state.playback_state.store(1, Ordering::Relaxed);
                    } else if !sink.empty() {
                        sink.pause();
                        state.playback_state.store(2, Ordering::Relaxed);
                    }
                }
                AudioCommand::Stop => {
                    sink = match Sink::try_new(&stream_handle) {
                        Ok(s) => s,
                        Err(e) => {
                            error!("Failed to recreate audio sink: {:?}", e);
                            continue;
                        }
                    };
                    sink.set_volume(current_volume);
                    sink.pause();

                    current_track_id = None;
                    if let Ok(mut lock) = state.current_track_id.write() {
                        *lock = None;
                    }
                    state.playback_state.store(0, Ordering::Relaxed);
                    state.position_ms.store(0, Ordering::Relaxed);
                    was_playing = false;
                }
                AudioCommand::Seek(pos) => {
                    let _ = sink.try_seek(pos);
                    state.position_ms.store(pos.as_millis() as u64, Ordering::Relaxed);
                }
                AudioCommand::SeekRelative(delta) => {
                    let current = sink.get_pos();
                    let current_secs = current.as_secs_f64();
                    let new_secs = (current_secs + delta).max(0.0);
                    let new_pos = Duration::from_secs_f64(new_secs);
                    let _ = sink.try_seek(new_pos);
                    state.position_ms.store(new_pos.as_millis() as u64, Ordering::Relaxed);
                }
                AudioCommand::SetVolume(vol) => {
                    current_volume = vol.clamp(0.0, 1.5);
                    sink.set_volume(current_volume);
                    state.volume_percent.store((current_volume * 100.0) as u8, Ordering::Relaxed);
                }
                AudioCommand::SetVolumeRelative(delta) => {
                    current_volume = (current_volume + delta).clamp(0.0, 1.5);
                    sink.set_volume(current_volume);
                    state.volume_percent.store((current_volume * 100.0) as u8, Ordering::Relaxed);
                }
            },
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Heartbeat
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                info!("Audio command channel disconnected, exiting audio thread");
                break;
            }
        }

        // Update position and state
        if was_playing {
            if !sink.empty() {
                let pos = sink.get_pos();
                state.position_ms.store(pos.as_millis() as u64, Ordering::Relaxed);
                if sink.is_paused() {
                    state.playback_state.store(2, Ordering::Relaxed);
                } else {
                    state.playback_state.store(1, Ordering::Relaxed);
                }
            } else {
                was_playing = false;
                state.playback_state.store(0, Ordering::Relaxed);
                state.position_ms.store(0, Ordering::Relaxed);

                if let Some(finished_id) = current_track_id.take() {
                    if let Ok(mut lock) = state.current_track_id.write() {
                        *lock = None;
                    }
                    let _ = event_tx.send(AudioEvent::TrackFinished(finished_id));
                }
            }
        }
    }
}
