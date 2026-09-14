use std::fs;
use std::path::PathBuf;
use anyhow::Result;

#[derive(Clone)]
pub struct TrackCache {
    cache_dir: PathBuf,
    enabled: bool,
}

impl TrackCache {
    pub fn new(cache_dir: PathBuf, enabled: bool) -> Self {
        let _ = fs::create_dir_all(&cache_dir);
        Self { cache_dir, enabled }
    }

    pub fn track_file_path(&self, track_id: &str) -> PathBuf {
        self.cache_dir.join(format!("{}.mp3", track_id))
    }

    pub fn get_path(&self, track_id: &str) -> Option<PathBuf> {
        let path = self.track_file_path(track_id);
        if path.exists() && fs::metadata(&path).map(|m| m.len() > 0).unwrap_or(false) {
            Some(path)
        } else {
            None
        }
    }

    pub fn has(&self, track_id: &str) -> bool {
        self.get_path(track_id).is_some()
    }

    pub fn clear(&self) -> Result<()> {
        if self.cache_dir.exists() {
            fs::remove_dir_all(&self.cache_dir)?;
            fs::create_dir_all(&self.cache_dir)?;
        }
        Ok(())
    }
}
