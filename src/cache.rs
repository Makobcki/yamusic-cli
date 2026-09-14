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
        if enabled {
            let _ = fs::create_dir_all(&cache_dir);
        }
        Self { cache_dir, enabled }
    }

    fn track_file_path(&self, track_id: &str) -> PathBuf {
        self.cache_dir.join(format!("{}.mp3", track_id))
    }

    pub fn get(&self, track_id: &str) -> Option<Vec<u8>> {
        if !self.enabled {
            return None;
        }
        let path = self.track_file_path(track_id);
        if path.exists() {
            match fs::read(&path) {
                Ok(data) if !data.is_empty() => Some(data),
                _ => None,
            }
        } else {
            None
        }
    }

    pub fn has(&self, track_id: &str) -> bool {
        if !self.enabled {
            return false;
        }
        let path = self.track_file_path(track_id);
        path.exists() && fs::metadata(&path).map(|m| m.len() > 0).unwrap_or(false)
    }

    pub fn put(&self, track_id: &str, data: &[u8]) -> Result<PathBuf> {
        if !self.enabled {
            return Ok(self.track_file_path(track_id));
        }
        let _ = fs::create_dir_all(&self.cache_dir);
        let path = self.track_file_path(track_id);
        let tmp_path = self.cache_dir.join(format!("{}.tmp.{}", track_id, std::process::id()));
        fs::write(&tmp_path, data)?;
        fs::rename(&tmp_path, &path)?;
        Ok(path)
    }

    pub fn clear(&self) -> Result<()> {
        if self.cache_dir.exists() {
            fs::remove_dir_all(&self.cache_dir)?;
            fs::create_dir_all(&self.cache_dir)?;
        }
        Ok(())
    }
}
