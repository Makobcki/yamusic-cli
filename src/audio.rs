use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};
use anyhow::{Context, Result};
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
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
    pub fn start(initial_volume: f32, fade_duration_ms: u64) -> Result<(Self, Receiver<AudioEvent>)> {
        let (cmd_tx, cmd_rx) = channel::<AudioCommand>();
        let (event_tx, event_rx) = channel::<AudioEvent>();
        let state = Arc::new(SharedAudioState::new(initial_volume));
        let state_clone = Arc::clone(&state);

        thread::Builder::new()
            .name("yamusic-audio".to_string())
            .spawn(move || {
                audio_thread_main(cmd_rx, event_tx, state_clone, initial_volume, fade_duration_ms);
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

struct CrossfadeSink {
    sink: Sink,
    from_volume: f32,
    start_time: Instant,
    duration: Duration,
}

enum FadeAction {
    None,
    FadeIn {
        from: f32,
        to: f32,
        start_time: Instant,
        duration: Duration,
    },
    FadeOutPause {
        from: f32,
        start_time: Instant,
        duration: Duration,
    },
    FadeOutStop {
        from: f32,
        start_time: Instant,
        duration: Duration,
    },
}

fn load_and_play_track(
    stream_handle: &OutputStreamHandle,
    path: &PathBuf,
    track_id: &str,
    start_pos: Duration,
    initial_volume: f32,
    state: &SharedAudioState,
) -> Result<(Sink, bool), ()> {
    let new_sink = match Sink::try_new(stream_handle) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to recreate audio sink: {:?}", e);
            return Err(());
        }
    };
    new_sink.set_volume(initial_volume);

    match std::fs::File::open(path) {
        Ok(file) => {
            let reader = std::io::BufReader::with_capacity(512 * 1024, file);
            match Decoder::new(reader) {
                Ok(decoder) => {
                    new_sink.append(decoder);
                    if start_pos > Duration::ZERO {
                        let _ = new_sink.try_seek(start_pos);
                    }
                    new_sink.play();
                    if let Ok(mut lock) = state.current_track_id.write() {
                        *lock = Some(track_id.to_string());
                    }
                    state.playback_state.store(1, Ordering::Relaxed);
                    Ok((new_sink, true))
                }
                Err(e) => {
                    error!("Failed to decode audio file {}: {:?}", path.display(), e);
                    if let Ok(mut lock) = state.current_track_id.write() {
                        *lock = None;
                    }
                    state.playback_state.store(0, Ordering::Relaxed);
                    Err(())
                }
            }
        }
        Err(e) => {
            error!("Failed to open audio file {}: {:?}", path.display(), e);
            Err(())
        }
    }
}

fn audio_thread_main(
    cmd_rx: Receiver<AudioCommand>,
    event_tx: Sender<AudioEvent>,
    state: Arc<SharedAudioState>,
    initial_volume: f32,
    fade_duration_ms: u64,
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

    let fade_duration = Duration::from_millis(fade_duration_ms);
    let mut target_volume = initial_volume.clamp(0.0, 1.5);
    let mut current_volume = target_volume;
    sink.set_volume(current_volume);

    let mut fading_sinks: Vec<CrossfadeSink> = Vec::new();
    let mut current_track_id: Option<String> = None;
    let mut was_playing = false;
    let mut fade_action = FadeAction::None;

    loop {
        let is_fading = !matches!(fade_action, FadeAction::None) || !fading_sinks.is_empty();
        let poll_timeout = if is_fading {
            Duration::from_millis(10)
        } else {
            Duration::from_millis(50)
        };

        match cmd_rx.recv_timeout(poll_timeout) {
            Ok(cmd) => match cmd {
                AudioCommand::PlayFile { path, track_id, start_pos } => {
                    if fade_duration > Duration::ZERO && was_playing && !sink.empty() && !sink.is_paused() {
                        // Concurrent crossfade: fade out old sink in background while new sink starts immediately
                        fading_sinks.push(CrossfadeSink {
                            sink,
                            from_volume: current_volume,
                            start_time: Instant::now(),
                            duration: fade_duration,
                        });
                        while fading_sinks.len() > 2 {
                            let oldest = fading_sinks.remove(0);
                            oldest.sink.stop();
                        }

                        match load_and_play_track(&stream_handle, &path, &track_id, start_pos, 0.0, &state) {
                            Ok((new_sink, playing)) => {
                                sink = new_sink;
                                current_track_id = Some(track_id);
                                was_playing = playing;
                                current_volume = 0.0;
                                fade_action = FadeAction::FadeIn {
                                    from: 0.0,
                                    to: target_volume,
                                    start_time: Instant::now(),
                                    duration: fade_duration,
                                };
                            }
                            Err(_) => {
                                sink = Sink::try_new(&stream_handle).unwrap_or_else(|_| {
                                    Sink::try_new(&stream_handle).expect("Sink recreation failed")
                                });
                                current_track_id = None;
                                was_playing = false;
                                current_volume = target_volume;
                                fade_action = FadeAction::None;
                            }
                        }
                    } else {
                        for f in fading_sinks.drain(..) {
                            f.sink.stop();
                        }
                        let initial_sink_vol = if fade_duration > Duration::ZERO { 0.0 } else { target_volume };
                        match load_and_play_track(&stream_handle, &path, &track_id, start_pos, initial_sink_vol, &state) {
                            Ok((new_sink, playing)) => {
                                sink = new_sink;
                                current_track_id = Some(track_id);
                                was_playing = playing;
                                if fade_duration > Duration::ZERO {
                                    current_volume = 0.0;
                                    fade_action = FadeAction::FadeIn {
                                        from: 0.0,
                                        to: target_volume,
                                        start_time: Instant::now(),
                                        duration: fade_duration,
                                    };
                                } else {
                                    current_volume = target_volume;
                                    fade_action = FadeAction::None;
                                }
                            }
                            Err(_) => {
                                sink = Sink::try_new(&stream_handle).unwrap_or_else(|_| {
                                    Sink::try_new(&stream_handle).expect("Sink recreation failed")
                                });
                                current_track_id = None;
                                was_playing = false;
                                current_volume = target_volume;
                                fade_action = FadeAction::None;
                            }
                        }
                    }
                }
                AudioCommand::Pause => {
                    for f in fading_sinks.drain(..) {
                        f.sink.stop();
                    }
                    if fade_duration > Duration::ZERO && was_playing && !sink.empty() && !sink.is_paused() {
                        if !matches!(fade_action, FadeAction::FadeOutPause { .. }) {
                            state.playback_state.store(2, Ordering::Relaxed);
                            fade_action = FadeAction::FadeOutPause {
                                from: current_volume,
                                start_time: Instant::now(),
                                duration: fade_duration,
                            };
                        }
                    } else {
                        sink.pause();
                        if was_playing {
                            state.playback_state.store(2, Ordering::Relaxed);
                        }
                        fade_action = FadeAction::None;
                        current_volume = target_volume;
                    }
                }
                AudioCommand::Resume => {
                    for f in fading_sinks.drain(..) {
                        f.sink.stop();
                    }
                    if !sink.empty() {
                        if fade_duration > Duration::ZERO {
                            if sink.is_paused() {
                                sink.set_volume(0.0);
                                sink.play();
                                current_volume = 0.0;
                                state.playback_state.store(1, Ordering::Relaxed);
                                fade_action = FadeAction::FadeIn {
                                    from: 0.0,
                                    to: target_volume,
                                    start_time: Instant::now(),
                                    duration: fade_duration,
                                };
                            } else if matches!(fade_action, FadeAction::FadeOutPause { .. }) {
                                state.playback_state.store(1, Ordering::Relaxed);
                                fade_action = FadeAction::FadeIn {
                                    from: current_volume,
                                    to: target_volume,
                                    start_time: Instant::now(),
                                    duration: fade_duration,
                                };
                            } else if matches!(fade_action, FadeAction::FadeIn { .. }) {
                                state.playback_state.store(1, Ordering::Relaxed);
                            }
                        } else {
                            sink.set_volume(target_volume);
                            sink.play();
                            current_volume = target_volume;
                            state.playback_state.store(1, Ordering::Relaxed);
                            fade_action = FadeAction::None;
                        }
                    }
                }
                AudioCommand::Toggle => {
                    if sink.is_paused() || matches!(fade_action, FadeAction::FadeOutPause { .. }) {
                        for f in fading_sinks.drain(..) {
                            f.sink.stop();
                        }
                        if !sink.empty() {
                            if fade_duration > Duration::ZERO {
                                if sink.is_paused() {
                                    sink.set_volume(0.0);
                                    sink.play();
                                    current_volume = 0.0;
                                    state.playback_state.store(1, Ordering::Relaxed);
                                    fade_action = FadeAction::FadeIn {
                                        from: 0.0,
                                        to: target_volume,
                                        start_time: Instant::now(),
                                        duration: fade_duration,
                                    };
                                } else {
                                    state.playback_state.store(1, Ordering::Relaxed);
                                    fade_action = FadeAction::FadeIn {
                                        from: current_volume,
                                        to: target_volume,
                                        start_time: Instant::now(),
                                        duration: fade_duration,
                                    };
                                }
                            } else {
                                sink.set_volume(target_volume);
                                sink.play();
                                current_volume = target_volume;
                                state.playback_state.store(1, Ordering::Relaxed);
                                fade_action = FadeAction::None;
                            }
                        }
                    } else if !sink.empty() {
                        for f in fading_sinks.drain(..) {
                            f.sink.stop();
                        }
                        if fade_duration > Duration::ZERO && was_playing && !sink.is_paused() {
                            state.playback_state.store(2, Ordering::Relaxed);
                            fade_action = FadeAction::FadeOutPause {
                                from: current_volume,
                                start_time: Instant::now(),
                                duration: fade_duration,
                            };
                        } else {
                            sink.pause();
                            state.playback_state.store(2, Ordering::Relaxed);
                            fade_action = FadeAction::None;
                            current_volume = target_volume;
                        }
                    }
                }
                AudioCommand::Stop => {
                    for f in fading_sinks.drain(..) {
                        f.sink.stop();
                    }
                    if fade_duration > Duration::ZERO && was_playing && !sink.empty() && !sink.is_paused() {
                        state.playback_state.store(0, Ordering::Relaxed);
                        fade_action = FadeAction::FadeOutStop {
                            from: current_volume,
                            start_time: Instant::now(),
                            duration: fade_duration,
                        };
                    } else {
                        sink = match Sink::try_new(&stream_handle) {
                            Ok(s) => s,
                            Err(e) => {
                                error!("Failed to recreate audio sink: {:?}", e);
                                continue;
                            }
                        };
                        current_volume = target_volume;
                        sink.set_volume(current_volume);
                        sink.pause();

                        current_track_id = None;
                        if let Ok(mut lock) = state.current_track_id.write() {
                            *lock = None;
                        }
                        state.playback_state.store(0, Ordering::Relaxed);
                        state.position_ms.store(0, Ordering::Relaxed);
                        was_playing = false;
                        fade_action = FadeAction::None;
                    }
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
                    target_volume = vol.clamp(0.0, 1.5);
                    state.volume_percent.store((target_volume * 100.0) as u8, Ordering::Relaxed);
                    match &mut fade_action {
                        FadeAction::None => {
                            current_volume = target_volume;
                            sink.set_volume(current_volume);
                        }
                        FadeAction::FadeIn { to, .. } => {
                            *to = target_volume;
                        }
                        _ => {}
                    }
                }
                AudioCommand::SetVolumeRelative(delta) => {
                    target_volume = (target_volume + delta).clamp(0.0, 1.5);
                    state.volume_percent.store((target_volume * 100.0) as u8, Ordering::Relaxed);
                    match &mut fade_action {
                        FadeAction::None => {
                            current_volume = target_volume;
                            sink.set_volume(current_volume);
                        }
                        FadeAction::FadeIn { to, .. } => {
                            *to = target_volume;
                        }
                        _ => {}
                    }
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

        // Process background fading sinks (crossfading old tracks)
        fading_sinks.retain_mut(|fading| {
            let elapsed = fading.start_time.elapsed();
            if elapsed >= fading.duration || fading.duration == Duration::ZERO {
                fading.sink.stop();
                false
            } else {
                let progress = (elapsed.as_secs_f32() / fading.duration.as_secs_f32()).clamp(0.0, 1.0);
                let vol = fading.from_volume * (1.0 - progress);
                fading.sink.set_volume(vol.clamp(0.0, 1.5));
                true
            }
        });

        // Process active sink fade
        let mut fade_completed = false;
        match &fade_action {
            FadeAction::FadeIn { from, to, start_time, duration } => {
                let elapsed = start_time.elapsed();
                if elapsed >= *duration || *duration == Duration::ZERO {
                    current_volume = *to;
                    sink.set_volume(current_volume);
                    fade_completed = true;
                } else {
                    let progress = (elapsed.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0);
                    current_volume = from + (to - from) * progress;
                    sink.set_volume(current_volume.clamp(0.0, 1.5));
                }
            }
            FadeAction::FadeOutPause { from, start_time, duration } => {
                let elapsed = start_time.elapsed();
                if elapsed >= *duration || *duration == Duration::ZERO {
                    current_volume = 0.0;
                    sink.set_volume(0.0);
                    sink.pause();
                    state.playback_state.store(2, Ordering::Relaxed);
                    fade_completed = true;
                } else {
                    let progress = (elapsed.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0);
                    current_volume = from * (1.0 - progress);
                    sink.set_volume(current_volume.clamp(0.0, 1.5));
                }
            }
            FadeAction::FadeOutStop { from, start_time, duration } => {
                let elapsed = start_time.elapsed();
                if elapsed >= *duration || *duration == Duration::ZERO {
                    sink = match Sink::try_new(&stream_handle) {
                        Ok(s) => s,
                        Err(e) => {
                            error!("Failed to recreate audio sink: {:?}", e);
                            continue;
                        }
                    };
                    current_volume = target_volume;
                    sink.set_volume(current_volume);
                    sink.pause();

                    current_track_id = None;
                    if let Ok(mut lock) = state.current_track_id.write() {
                        *lock = None;
                    }
                    state.playback_state.store(0, Ordering::Relaxed);
                    state.position_ms.store(0, Ordering::Relaxed);
                    was_playing = false;
                    fade_completed = true;
                } else {
                    let progress = (elapsed.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0);
                    current_volume = from * (1.0 - progress);
                    sink.set_volume(current_volume.clamp(0.0, 1.5));
                }
            }
            FadeAction::None => {}
        }

        if fade_completed {
            fade_action = FadeAction::None;
        }

        // Update position and state
        if was_playing {
            if !sink.empty() {
                let pos = sink.get_pos();
                state.position_ms.store(pos.as_millis() as u64, Ordering::Relaxed);
                if sink.is_paused() {
                    state.playback_state.store(2, Ordering::Relaxed);
                } else if !matches!(fade_action, FadeAction::FadeOutPause { .. }) {
                    state.playback_state.store(1, Ordering::Relaxed);
                }
            } else {
                was_playing = false;
                state.playback_state.store(0, Ordering::Relaxed);
                state.position_ms.store(0, Ordering::Relaxed);
                fade_action = FadeAction::None;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shared_audio_state() {
        let state = SharedAudioState::new(0.8);
        assert_eq!(state.state(), PlaybackState::Stopped);
        assert!((state.volume() - 0.8).abs() < 0.01);
        assert_eq!(state.position(), Duration::from_millis(0));
        assert_eq!(state.current_track_id(), None);

        state.playback_state.store(1, Ordering::Relaxed);
        assert_eq!(state.state(), PlaybackState::Playing);

        state.playback_state.store(2, Ordering::Relaxed);
        assert_eq!(state.state(), PlaybackState::Paused);

        state.position_ms.store(5000, Ordering::Relaxed);
        assert_eq!(state.position(), Duration::from_millis(5000));

        if let Ok(mut lock) = state.current_track_id.write() {
            *lock = Some("12345".to_string());
        }
        assert_eq!(state.current_track_id(), Some("12345".to_string()));
    }
}
