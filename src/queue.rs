use rand::seq::SliceRandom;
use rand::thread_rng;
use crate::types::{LoopMode, Track};

#[derive(Debug, Clone)]
pub enum QueueSource {
    MyWave {
        session_id: String,
        batch_id: Option<String>,
    },
    Liked,
    Playlist {
        kind: u64,
        uid: u64,
        title: String,
    },
    Search {
        query: String,
    },
    Custom,
}

pub struct PlayQueue {
    tracks: Vec<Track>,
    current_index: Option<usize>,
    source_name: String,
    source: QueueSource,
    loop_mode: LoopMode,
    shuffle: bool,
    shuffled_indices: Vec<usize>,
    shuffle_pos: usize,
}

impl PlayQueue {
    pub fn new() -> Self {
        Self {
            tracks: Vec::new(),
            current_index: None,
            source_name: "Empty".to_string(),
            source: QueueSource::Custom,
            loop_mode: LoopMode::Off,
            shuffle: false,
            shuffled_indices: Vec::new(),
            shuffle_pos: 0,
        }
    }

    pub fn source_name(&self) -> &str {
        &self.source_name
    }

    pub fn source(&self) -> &QueueSource {
        &self.source
    }

    pub fn source_mut(&mut self) -> &mut QueueSource {
        &mut self.source
    }

    pub fn loop_mode(&self) -> LoopMode {
        self.loop_mode
    }

    pub fn set_loop_mode(&mut self, mode: LoopMode) {
        self.loop_mode = mode;
    }

    pub fn is_shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn set_shuffle(&mut self, enable: bool) {
        self.shuffle = enable;
        if enable {
            self.rebuild_shuffle_indices();
        }
    }

    pub fn toggle_shuffle(&mut self) -> bool {
        let new_state = !self.shuffle;
        self.set_shuffle(new_state);
        new_state
    }

    fn rebuild_shuffle_indices(&mut self) {
        let len = self.tracks.len();
        if len == 0 {
            self.shuffled_indices.clear();
            self.shuffle_pos = 0;
            return;
        }

        let mut indices: Vec<usize> = (0..len).collect();
        let mut rng = thread_rng();
        indices.shuffle(&mut rng);

        // Put currently playing track at the start if any
        if let Some(curr) = self.current_index {
            if let Some(pos) = indices.iter().position(|&x| x == curr) {
                indices.remove(pos);
                indices.insert(0, curr);
            }
        }

        self.shuffled_indices = indices;
        self.shuffle_pos = 0;
    }

    pub fn set_tracks(
        &mut self,
        source_name: String,
        source: QueueSource,
        tracks: Vec<Track>,
        start_index: usize,
    ) {
        self.source_name = source_name;
        self.source = source;
        self.tracks = tracks;
        if self.tracks.is_empty() {
            self.current_index = None;
            self.shuffled_indices.clear();
            self.shuffle_pos = 0;
        } else {
            let idx = start_index.min(self.tracks.len() - 1);
            self.current_index = Some(idx);
            if self.shuffle {
                self.rebuild_shuffle_indices();
            }
        }
    }

    pub fn append_tracks(&mut self, new_tracks: Vec<Track>) {
        if new_tracks.is_empty() {
            return;
        }
        let old_len = self.tracks.len();
        self.tracks.extend(new_tracks);

        if self.current_index.is_none() && !self.tracks.is_empty() {
            self.current_index = Some(0);
        }

        if self.shuffle {
            let mut new_indices: Vec<usize> = (old_len..self.tracks.len()).collect();
            let mut rng = thread_rng();
            new_indices.shuffle(&mut rng);
            self.shuffled_indices.extend(new_indices);
        }
    }

    pub fn current_track(&self) -> Option<&Track> {
        self.current_index.and_then(|idx| self.tracks.get(idx))
    }

    pub fn current_index(&self) -> Option<usize> {
        self.current_index
    }

    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn next(&mut self) -> Option<(usize, &Track)> {
        if self.tracks.is_empty() {
            return None;
        }

        if self.loop_mode == LoopMode::Track {
            if let Some(idx) = self.current_index {
                return self.tracks.get(idx).map(|t| (idx, t));
            }
        }

        if self.shuffle {
            if self.shuffled_indices.is_empty() {
                self.rebuild_shuffle_indices();
            }
            if self.shuffle_pos + 1 < self.shuffled_indices.len() {
                self.shuffle_pos += 1;
                let next_idx = self.shuffled_indices[self.shuffle_pos];
                self.current_index = Some(next_idx);
                return self.tracks.get(next_idx).map(|t| (next_idx, t));
            } else if self.loop_mode == LoopMode::All {
                self.rebuild_shuffle_indices();
                self.shuffle_pos = 0;
                let next_idx = self.shuffled_indices[0];
                self.current_index = Some(next_idx);
                return self.tracks.get(next_idx).map(|t| (next_idx, t));
            } else {
                return None;
            }
        }

        let curr = self.current_index.unwrap_or(0);
        if curr + 1 < self.tracks.len() {
            let next_idx = curr + 1;
            self.current_index = Some(next_idx);
            self.tracks.get(next_idx).map(|t| (next_idx, t))
        } else if self.loop_mode == LoopMode::All && !self.tracks.is_empty() {
            self.current_index = Some(0);
            self.tracks.get(0).map(|t| (0, t))
        } else {
            None
        }
    }

    pub fn previous(&mut self) -> Option<(usize, &Track)> {
        if self.tracks.is_empty() {
            return None;
        }

        if self.shuffle {
            if self.shuffle_pos > 0 {
                self.shuffle_pos -= 1;
                let prev_idx = self.shuffled_indices[self.shuffle_pos];
                self.current_index = Some(prev_idx);
                return self.tracks.get(prev_idx).map(|t| (prev_idx, t));
            } else if let Some(idx) = self.current_index {
                return self.tracks.get(idx).map(|t| (idx, t));
            }
        }

        let curr = self.current_index.unwrap_or(0);
        if curr > 0 {
            let prev_idx = curr - 1;
            self.current_index = Some(prev_idx);
            self.tracks.get(prev_idx).map(|t| (prev_idx, t))
        } else if self.loop_mode == LoopMode::All && !self.tracks.is_empty() {
            let last_idx = self.tracks.len() - 1;
            self.current_index = Some(last_idx);
            self.tracks.get(last_idx).map(|t| (last_idx, t))
        } else {
            self.tracks.get(curr).map(|t| (curr, t))
        }
    }

    pub fn jump(&mut self, index: usize) -> Option<&Track> {
        if index < self.tracks.len() {
            self.current_index = Some(index);
            if self.shuffle {
                if let Some(pos) = self.shuffled_indices.iter().position(|&x| x == index) {
                    self.shuffle_pos = pos;
                }
            }
            self.tracks.get(index)
        } else {
            None
        }
    }

    pub fn tracks_remaining(&self) -> usize {
        match self.current_index {
            Some(idx) => self.tracks.len().saturating_sub(idx + 1),
            None => self.tracks.len(),
        }
    }

    pub fn is_my_wave(&self) -> bool {
        matches!(self.source, QueueSource::MyWave { .. })
    }

    pub fn clear(&mut self) {
        self.tracks.clear();
        self.current_index = None;
        self.shuffled_indices.clear();
        self.shuffle_pos = 0;
        self.source_name = "Empty".to_string();
        self.source = QueueSource::Custom;
    }
}
