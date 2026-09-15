use std::fs;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub token: String,
    #[serde(default = "default_volume")]
    pub volume: f32,
    #[serde(default = "default_cache_enabled")]
    pub cache_enabled: bool,
    #[serde(default)]
    pub cache_dir: Option<PathBuf>,
    #[serde(default = "default_bitrate")]
    pub bitrate: u32,
    #[serde(default = "default_mpris_enabled")]
    pub mpris_enabled: bool,
    #[serde(default = "default_fade_duration_ms")]
    pub fade_duration_ms: u64,
}

fn default_volume() -> f32 {
    0.7
}

fn default_cache_enabled() -> bool {
    true
}

fn default_bitrate() -> u32 {
    320
}

fn default_mpris_enabled() -> bool {
    true
}

fn default_fade_duration_ms() -> u64 {
    200
}

impl Default for Config {
    fn default() -> Self {
        Self {
            token: String::new(),
            volume: default_volume(),
            cache_enabled: default_cache_enabled(),
            cache_dir: None,
            bitrate: default_bitrate(),
            mpris_enabled: default_mpris_enabled(),
            fade_duration_ms: default_fade_duration_ms(),
        }
    }
}

impl Config {
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .map(|p| p.join("yamusic-cli"))
            .unwrap_or_else(|| PathBuf::from(".config/yamusic-cli"))
    }

    pub fn config_file_path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    pub fn default_cache_dir() -> PathBuf {
        dirs::cache_dir()
            .map(|p| p.join("yamusic-cli").join("tracks"))
            .unwrap_or_else(|| PathBuf::from(".cache/yamusic-cli/tracks"))
    }

    pub fn get_cache_dir(&self) -> PathBuf {
        self.cache_dir
            .clone()
            .unwrap_or_else(Self::default_cache_dir)
    }

    pub fn socket_path() -> PathBuf {
        if let Some(runtime_dir) = dirs::runtime_dir() {
            let yamusic_dir = runtime_dir.join("yamusic-cli");
            let _ = fs::create_dir_all(&yamusic_dir);
            yamusic_dir.join("yamusic-cli.sock")
        } else {
            let uid = std::env::var("UID").unwrap_or_else(|_| "1000".to_string());
            PathBuf::from(format!("/tmp/yamusic-cli-{}.sock", uid))
        }
    }

    pub fn load(cli_token: Option<&str>) -> anyhow::Result<Self> {
        let mut config = Self::default();

        // 1. Try ~/.config/yamusic-cli/config.toml
        let config_path = Self::config_file_path();
        if config_path.exists() {
            if let Ok(content) = fs::read_to_string(&config_path) {
                if let Ok(parsed) = toml::from_str::<Config>(&content) {
                    config = parsed;
                }
            }
        }

        // 1b. Fallback to legacy ~/.config/yamusic/config.toml
        if config.token.is_empty() {
            if let Some(cfg_dir) = dirs::config_dir() {
                let legacy_path = cfg_dir.join("yamusic").join("config.toml");
                if legacy_path.exists() {
                    if let Ok(content) = fs::read_to_string(&legacy_path) {
                        if let Ok(parsed) = toml::from_str::<Config>(&content) {
                            config = parsed;
                        }
                    }
                }
            }
        }

        // 2. If token is still empty, try ~/.config/yamusic-tui/config.yaml
        if config.token.is_empty() {
            if let Some(home) = dirs::home_dir() {
                let yamusic_tui_path = home.join(".config/yamusic-tui/config.yaml");
                if yamusic_tui_path.exists() {
                    if let Ok(content) = fs::read_to_string(&yamusic_tui_path) {
                        for line in content.lines() {
                            let trimmed = line.trim();
                            if trimmed.starts_with("token:") {
                                let token_val = trimmed
                                    .trim_start_matches("token:")
                                    .trim()
                                    .trim_matches('"')
                                    .trim_matches('\'');
                                if !token_val.is_empty() {
                                    config.token = token_val.to_string();
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }

        // 3. Environment variable YANDEX_MUSIC_TOKEN
        if let Ok(env_token) = std::env::var("YANDEX_MUSIC_TOKEN") {
            if !env_token.trim().is_empty() {
                config.token = env_token.trim().to_string();
            }
        }

        // 4. CLI argument --token
        if let Some(token) = cli_token {
            if !token.trim().is_empty() {
                config.token = token.trim().to_string();
            }
        }

        Ok(config)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let dir = Self::config_dir();
        fs::create_dir_all(&dir)?;
        let content = toml::to_string_pretty(self)?;
        fs::write(dir.join("config.toml"), content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = Config::default();
        assert_eq!(cfg.fade_duration_ms, 200);
        assert_eq!(cfg.volume, 0.7);
        assert!(cfg.cache_enabled);
        assert!(cfg.mpris_enabled);
    }

    #[test]
    fn test_parse_config_with_custom_fade() {
        let toml_str = r#"
            token = "test_token"
            volume = 0.5
            fade_duration_ms = 350
        "#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.token, "test_token");
        assert_eq!(cfg.volume, 0.5);
        assert_eq!(cfg.fade_duration_ms, 350);
        assert!(cfg.cache_enabled); // default
    }

    #[test]
    fn test_parse_config_with_disabled_fade() {
        let toml_str = r#"
            token = "test_token"
            fade_duration_ms = 0
        "#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.fade_duration_ms, 0);
    }
}

