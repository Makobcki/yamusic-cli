use std::collections::{HashSet, VecDeque};
use rand::seq::SliceRandom;
use rand::thread_rng;
use crate::types::{LoopMode, Track};

const MAX_RECENT_HISTORY: usize = 200;

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
    recently_played: VecDeque<String>,
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
            recently_played: VecDeque::with_capacity(MAX_RECENT_HISTORY),
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

    pub fn record_played_index(&mut self, idx: usize) {
        if let Some(t) = self.tracks.get(idx) {
            let id = t.id.clone();
            if self.recently_played.back().map(|s| s.as_str()) == Some(&id) {
                return;
            }
            if self.recently_played.len() >= MAX_RECENT_HISTORY {
                self.recently_played.pop_front();
            }
            self.recently_played.push_back(id);
        }
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
            self.record_played_index(idx);
            if self.shuffle {
                self.rebuild_shuffle_indices();
            }
        }
    }

    pub fn append_tracks(&mut self, new_tracks: Vec<Track>) {
        if new_tracks.is_empty() {
            return;
        }

        // Deduplicate tracks for MyWave and dynamic queues
        let tracks_to_add: Vec<Track> = if self.is_my_wave() {
            let existing_ids: HashSet<String> = self.tracks.iter().map(|t| t.id.clone()).collect();
            let recent_set: HashSet<String> = self.recently_played.iter().cloned().collect();
            let mut seen = HashSet::new();

            // First try filtering against both existing queue and recent history
            let fresh: Vec<Track> = new_tracks
                .iter()
                .filter(|t| !existing_ids.contains(&t.id) && !recent_set.contains(&t.id) && seen.insert(t.id.clone()))
                .cloned()
                .collect();

            if !fresh.is_empty() {
                fresh
            } else {
                // Fallback: at least avoid duplicates in current active queue
                seen.clear();
                new_tracks
                    .into_iter()
                    .filter(|t| !existing_ids.contains(&t.id) && seen.insert(t.id.clone()))
                    .collect()
            }
        } else {
            new_tracks
        };

        if tracks_to_add.is_empty() {
            return;
        }

        let old_len = self.tracks.len();
        self.tracks.extend(tracks_to_add);

        if self.current_index.is_none() && !self.tracks.is_empty() {
            self.current_index = Some(0);
            self.record_played_index(0);
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
                self.record_played_index(next_idx);
                return self.tracks.get(next_idx).map(|t| (next_idx, t));
            } else if self.loop_mode == LoopMode::All {
                self.rebuild_shuffle_indices();
                self.shuffle_pos = 0;
                let next_idx = self.shuffled_indices[0];
                self.current_index = Some(next_idx);
                self.record_played_index(next_idx);
                return self.tracks.get(next_idx).map(|t| (next_idx, t));
            } else {
                return None;
            }
        }

        let curr = self.current_index.unwrap_or(0);
        if curr + 1 < self.tracks.len() {
            let next_idx = curr + 1;
            self.current_index = Some(next_idx);
            self.record_played_index(next_idx);
            self.tracks.get(next_idx).map(|t| (next_idx, t))
        } else if self.loop_mode == LoopMode::All && !self.tracks.is_empty() {
            self.current_index = Some(0);
            self.record_played_index(0);
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
                self.record_played_index(prev_idx);
                return self.tracks.get(prev_idx).map(|t| (prev_idx, t));
            } else if let Some(idx) = self.current_index {
                return self.tracks.get(idx).map(|t| (idx, t));
            }
        }

        let curr = self.current_index.unwrap_or(0);
        if curr > 0 {
            let prev_idx = curr - 1;
            self.current_index = Some(prev_idx);
            self.record_played_index(prev_idx);
            self.tracks.get(prev_idx).map(|t| (prev_idx, t))
        } else if self.loop_mode == LoopMode::All && !self.tracks.is_empty() {
            let last_idx = self.tracks.len() - 1;
            self.current_index = Some(last_idx);
            self.record_played_index(last_idx);
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
            self.record_played_index(index);
            self.tracks.get(index)
        } else {
            None
        }
    }

    pub fn tracks_remaining(&self) -> usize {
        if self.shuffle {
            self.shuffled_indices.len().saturating_sub(self.shuffle_pos + 1)
        } else {
            match self.current_index {
                Some(idx) => self.tracks.len().saturating_sub(idx + 1),
                None => self.tracks.len(),
            }
        }
    }

    pub fn remaining_rotor_keys(&self, limit: usize) -> Vec<String> {
        if self.shuffle {
            self.shuffled_indices
                .iter()
                .skip(self.shuffle_pos + 1)
                .take(limit)
                .filter_map(|&idx| self.tracks.get(idx))
                .map(|t| t.rotor_key())
                .collect()
        } else {
            let start = self.current_index.map(|i| i + 1).unwrap_or(0);
            self.tracks
                .iter()
                .skip(start)
                .take(limit)
                .map(|t| t.rotor_key())
                .collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Album, Artist};

    fn make_test_track(id: &str, title: &str) -> Track {
        Track {
            id: id.to_string(),
            real_id: None,
            title: title.to_string(),
            version: None,
            available: true,
            duration_ms: 180000,
            cover_uri: None,
            artists: vec![Artist {
                id: 1,
                name: "Artist".to_string(),
                various: false,
                composer: false,
            }],
            albums: vec![Album {
                id: 10,
                title: "Album".to_string(),
                year: None,
                cover_uri: None,
                genre: None,
            }],
        }
    }

    #[test]
    fn test_queue_deduplication_in_wave() {
        let mut q = PlayQueue::new();
        let t1 = make_test_track("1", "Track 1");
        let t2 = make_test_track("2", "Track 2");
        let t3 = make_test_track("3", "Track 3");

        q.set_tracks(
            "Моя волна".to_string(),
            QueueSource::MyWave {
                session_id: "s1".to_string(),
                batch_id: Some("b1".to_string()),
            },
            vec![t1.clone(), t2.clone()],
            0,
        );

        assert_eq!(q.len(), 2);

        // Appending t2 (already in queue) and t3 (new)
        q.append_tracks(vec![t2, t3]);
        assert_eq!(q.len(), 3);
        assert_eq!(q.tracks()[2].id, "3");
    }

    #[test]
    fn test_tracks_remaining_with_shuffle() {
        let mut q = PlayQueue::new();
        let tracks: Vec<Track> = (1..=10).map(|i| make_test_track(&i.to_string(), &format!("Track {}", i))).collect();
        q.set_tracks("Test".to_string(), QueueSource::Custom, tracks, 0);
        q.set_shuffle(true);

        assert_eq!(q.tracks_remaining(), 9);
        let _ = q.next();
        assert_eq!(q.tracks_remaining(), 8);
    }
}
