use serde::{Deserialize, Deserializer, Serialize};

pub fn deserialize_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrInt {
        String(String),
        Int(i64),
        UInt(u64),
    }
    match StringOrInt::deserialize(deserializer)? {
        StringOrInt::String(s) => Ok(s),
        StringOrInt::Int(i) => Ok(i.to_string()),
        StringOrInt::UInt(u) => Ok(u.to_string()),
    }
}

pub fn deserialize_opt_id<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrInt {
        String(String),
        Int(i64),
        UInt(u64),
    }
    match Option::<StringOrInt>::deserialize(deserializer)? {
        Some(StringOrInt::String(s)) => Ok(Some(s)),
        Some(StringOrInt::Int(i)) => Ok(Some(i.to_string())),
        Some(StringOrInt::UInt(u)) => Ok(Some(u.to_string())),
        None => Ok(None),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artist {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub various: bool,
    #[serde(default)]
    pub composer: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Album {
    pub id: u64,
    pub title: String,
    #[serde(default)]
    pub year: Option<u32>,
    #[serde(rename = "coverUri", default)]
    pub cover_uri: Option<String>,
    #[serde(default)]
    pub genre: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    #[serde(deserialize_with = "deserialize_id")]
    pub id: String,
    #[serde(rename = "realId", default, deserialize_with = "deserialize_opt_id")]
    pub real_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub available: bool,
    #[serde(rename = "durationMs", default)]
    pub duration_ms: u64,
    #[serde(rename = "coverUri", default)]
    pub cover_uri: Option<String>,
    #[serde(default)]
    pub artists: Vec<Artist>,
    #[serde(default)]
    pub albums: Vec<Album>,
}

impl Track {
    pub fn artists_str(&self) -> String {
        if self.artists.is_empty() {
            "Unknown Artist".to_string()
        } else {
            self.artists
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        }
    }

    pub fn album_title(&self) -> &str {
        self.albums.first().map(|a| a.title.as_str()).unwrap_or("Unknown Album")
    }

    pub fn album_id(&self) -> Option<u64> {
        self.albums.first().map(|a| a.id)
    }

    pub fn rotor_key(&self) -> String {
        match self.album_id() {
            Some(album_id) => format!("{}:{}", self.id, album_id),
            None => self.id.clone(),
        }
    }

    pub fn cover_url(&self, size: u32) -> Option<String> {
        self.cover_uri.as_ref().map(|uri| {
            let base = if uri.ends_with("%%") {
                &uri[..uri.len() - 2]
            } else {
                uri.as_str()
            };
            format!("https://{}{}x{}", base, size, size)
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistTrackItem {
    pub id: Option<u64>,
    pub track: Track,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub uid: u64,
    pub kind: u64,
    pub title: String,
    #[serde(rename = "trackCount", default)]
    pub track_count: usize,
    #[serde(default)]
    pub tracks: Vec<PlaylistTrackItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LikedTrackItem {
    #[serde(deserialize_with = "deserialize_id")]
    pub id: String,
    #[serde(rename = "albumId", default, deserialize_with = "deserialize_opt_id")]
    pub album_id: Option<String>,
    #[serde(default)]
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LikesLibrary {
    pub uid: u64,
    #[serde(default)]
    pub tracks: Vec<LikedTrackItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LikesDesc {
    pub library: LikesLibrary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserAccount {
    pub uid: u64,
    #[serde(rename = "displayName", default)]
    pub display_name: Option<String>,
    #[serde(rename = "fullName", default)]
    pub full_name: Option<String>,
    #[serde(default)]
    pub login: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserStatus {
    pub account: UserAccount,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackDownloadInfo {
    pub codec: String,
    #[serde(rename = "downloadInfoUrl")]
    pub download_info_url: String,
    #[serde(rename = "bitrateInKbps", default)]
    pub bitrate_in_kbps: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullDownloadInfo {
    pub host: String,
    pub path: String,
    pub ts: String,
    pub s: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResultTracks {
    #[serde(default)]
    pub total: usize,
    #[serde(default)]
    pub results: Vec<Track>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub tracks: Option<SearchResultTracks>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotorSequenceItem {
    pub track: Track,
    #[serde(default)]
    pub liked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotorStationTracks {
    #[serde(rename = "radioSessionId", default)]
    pub radio_session_id: Option<String>,
    #[serde(rename = "batchId", default)]
    pub batch_id: Option<String>,
    #[serde(default)]
    pub sequence: Vec<RotorSequenceItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotorFeedbackEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(rename = "trackId", skip_serializing_if = "Option::is_none")]
    pub track_id: Option<String>,
    #[serde(rename = "totalPlayedSeconds", skip_serializing_if = "Option::is_none")]
    pub total_played_seconds: Option<f64>,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotorFeedback {
    pub event: RotorFeedbackEvent,
    #[serde(rename = "batchId", skip_serializing_if = "Option::is_none")]
    pub batch_id: Option<String>,
    #[serde(default = "default_feedback_from")]
    pub from: String,
}

fn default_feedback_from() -> String {
    "yamusic-cli".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub result: Option<T>,
    pub error: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaybackState {
    Playing,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoopMode {
    Off,
    All,
    Track,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerStatus {
    pub state: PlaybackState,
    pub current_track: Option<Track>,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub volume: f32,
    pub is_liked: bool,
    pub loop_mode: LoopMode,
    pub shuffle: bool,
    pub source_name: String,
    pub queue_index: usize,
    pub queue_len: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum IpcRequest {
    Status,
    Play { query: Option<String>, track_id: Option<String> },
    Pause,
    Toggle,
    Stop,
    Next,
    Prev,
    Seek { seconds: f64, relative: bool },
    Volume { value: Option<f32>, delta: Option<f32> },
    Like { track_id: Option<String> },
    Unlike { track_id: Option<String> },
    Playlists,
    PlayPlaylist { name_or_kind: String, shuffle: Option<bool> },
    PlayWave,
    PlayLiked { shuffle: Option<bool> },
    Search { query: String },
    Queue,
    Jump { index: usize },
    Shuffle { enable: Option<bool> },
    Loop { mode: Option<LoopMode> },
    QuitDaemon,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcResponse {
    pub success: bool,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

impl IpcResponse {
    pub fn ok(message: impl Into<String>, data: Option<serde_json::Value>) -> Self {
        Self {
            success: true,
            message: Some(message.into()),
            data,
        }
    }

    pub fn ok_data(data: serde_json::Value) -> Self {
        Self {
            success: true,
            message: None,
            data: Some(data),
        }
    }

    pub fn err(message: impl Into<String>) -> Self {
        Self {
            success: false,
            message: Some(message.into()),
            data: None,
        }
    }
}
