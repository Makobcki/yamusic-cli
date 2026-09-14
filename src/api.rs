use anyhow::{bail, Context, Result};
use md5::{Digest, Md5};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, USER_AGENT};
use serde_json::json;
use crate::types::*;

const BASE_URL: &str = "https://api.music.yandex.net";
const DOWNLOAD_SECRET_SALT: &str = "XGRlBW9FXlekgbPrRHuSiA";

#[derive(Clone)]
pub struct YaMusicClient {
    http: reqwest::Client,
    token: String,
}

impl YaMusicClient {
    pub fn new(token: String) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static("okhttp/4.12.0"),
        );
        headers.insert(
            "x-Yandex-Music-Client",
            HeaderValue::from_static("YandexMusicAndroid/24024312"),
        );
        let auth_val = format!("OAuth {}", token.trim());
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&auth_val).context("Invalid token format for header")?,
        );

        let http = reqwest::Client::builder()
            .default_headers(headers)
            .build()?;

        Ok(Self { http, token })
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub async fn get_status(&self) -> Result<UserStatus> {
        let url = format!("{}/account/status", BASE_URL);
        let resp: ApiResponse<UserStatus> = self.http.get(&url).send().await?.json().await?;
        resp.result.context("Empty user status in API response")
    }

    pub async fn get_liked_track_ids(&self, uid: u64) -> Result<Vec<String>> {
        let url = format!("{}/users/{}/likes/tracks", BASE_URL, uid);
        let resp: ApiResponse<LikesDesc> = self.http.get(&url).send().await?.json().await?;
        let desc = resp.result.context("Empty likes response")?;
        Ok(desc.library.tracks.into_iter().map(|item| item.id).collect())
    }

    pub async fn get_tracks(&self, track_ids: &[String]) -> Result<Vec<Track>> {
        if track_ids.is_empty() {
            return Ok(vec![]);
        }

        let mut all_tracks = Vec::new();

        // Process in chunks of 50 to avoid URL/body length limits
        for chunk in track_ids.chunks(50) {
            let mut params = Vec::new();
            for id in chunk {
                params.push(("track-ids", id.as_str()));
            }
            params.push(("with-positions", "false"));

            let url = format!("{}/tracks", BASE_URL);
            let val: serde_json::Value = self
                .http
                .post(&url)
                .form(&params)
                .send()
                .await?
                .json()
                .await?;

            if let Some(res) = val.get("result") {
                if let Ok(tracks) = serde_json::from_value::<Vec<Track>>(res.clone()) {
                    all_tracks.extend(tracks);
                }
            }
        }

        Ok(all_tracks)
    }

    pub async fn get_playlists(&self, uid: u64) -> Result<Vec<Playlist>> {
        let url = format!("{}/users/{}/playlists/list", BASE_URL, uid);
        let resp: ApiResponse<Vec<Playlist>> = self.http.get(&url).send().await?.json().await?;
        Ok(resp.result.unwrap_or_default())
    }

    pub async fn get_playlist_tracks(&self, uid: u64, kind: u64) -> Result<Vec<Track>> {
        let url = format!("{}/users/{}/playlists", BASE_URL, uid);
        let val: serde_json::Value = self
            .http
            .get(&url)
            .query(&[
                ("kinds", kind.to_string()),
                ("mixed", "false".to_string()),
                ("rich-tracks", "true".to_string()),
            ])
            .send()
            .await?
            .json()
            .await?;

        if let Some(res) = val.get("result") {
            if let Ok(mut playlists) = serde_json::from_value::<Vec<Playlist>>(res.clone()) {
                if let Some(pl) = playlists.pop() {
                    let tracks: Vec<Track> = pl.tracks.into_iter().map(|item| item.track).collect();
                    return Ok(tracks);
                }
            }
        }

        // Fallback to /users/{uid}/playlists/{kind}
        let direct_url = format!("{}/users/{}/playlists/{}", BASE_URL, uid, kind);
        let direct_val: serde_json::Value = self.http.get(&direct_url).send().await?.json().await?;
        if let Some(res) = direct_val.get("result") {
            if let Ok(pl) = serde_json::from_value::<Playlist>(res.clone()) {
                let tracks: Vec<Track> = pl.tracks.into_iter().map(|item| item.track).collect();
                return Ok(tracks);
            }
        }

        bail!("Playlist {} not found", kind)
    }

    pub async fn search(&self, query: &str) -> Result<Vec<Track>> {
        let url = format!("{}/search", BASE_URL);
        let val: serde_json::Value = self
            .http
            .get(&url)
            .query(&[
                ("text", query),
                ("type", "track"),
                ("page", "0"),
            ])
            .send()
            .await?
            .json()
            .await?;

        let result_val = val.get("result").context("Search returned empty result")?;
        let result: SearchResult = serde_json::from_value(result_val.clone())?;
        let tracks = result.tracks.map(|t| t.results).unwrap_or_default();
        Ok(tracks)
    }

    pub async fn like_track(&self, uid: u64, track_id: &str) -> Result<()> {
        let url = format!("{}/users/{}/likes/tracks/add-multiple", BASE_URL, uid);
        let params = [("track-ids", track_id)];
        let resp: ApiResponse<serde_json::Value> = self
            .http
            .post(&url)
            .form(&params)
            .send()
            .await?
            .json()
            .await?;
        if let Some(err) = resp.error {
            bail!("Error liking track: {:?}", err);
        }
        Ok(())
    }

    pub async fn unlike_track(&self, uid: u64, track_id: &str) -> Result<()> {
        let url = format!("{}/users/{}/likes/tracks/remove", BASE_URL, uid);
        let params = [("track-ids", track_id)];
        let resp: ApiResponse<serde_json::Value> = self
            .http
            .post(&url)
            .form(&params)
            .send()
            .await?
            .json()
            .await?;
        if let Some(err) = resp.error {
            bail!("Error unliking track: {:?}", err);
        }
        Ok(())
    }

    pub async fn resolve_download_url(&self, track_id: &str, preferred_bitrate: u32) -> Result<String> {
        let info_url = format!("{}/tracks/{}/download-info", BASE_URL, track_id);
        let resp: ApiResponse<Vec<TrackDownloadInfo>> = self.http.get(&info_url).send().await?.json().await?;
        let options = resp.result.context("No download info options returned")?;

        // Filter mp3 options
        let mp3_options: Vec<_> = options
            .into_iter()
            .filter(|opt| opt.codec == "mp3")
            .collect();

        if mp3_options.is_empty() {
            bail!("No MP3 download option available for track {}", track_id);
        }

        // Pick closest to preferred_bitrate or highest
        let best_opt = mp3_options
            .into_iter()
            .max_by_key(|opt| {
                if opt.bitrate_in_kbps <= preferred_bitrate {
                    opt.bitrate_in_kbps
                } else {
                    preferred_bitrate - (opt.bitrate_in_kbps - preferred_bitrate)
                }
            })
            .context("Failed to select download option")?;

        // Fetch XML/JSON download info
        let dl_info_url = format!("{}&format=json", best_opt.download_info_url);
        let full_info: FullDownloadInfo = self.http.get(&dl_info_url).send().await?.json().await?;

        // Compute MD5 hash: md5(DOWNLOAD_SECRET_SALT + path[1..] + s)
        let path_without_leading_slash = if full_info.path.starts_with('/') {
            &full_info.path[1..]
        } else {
            &full_info.path
        };

        let sign_input = format!(
            "{}{}{}",
            DOWNLOAD_SECRET_SALT, path_without_leading_slash, full_info.s
        );
        let mut hasher = Md5::new();
        hasher.update(sign_input.as_bytes());
        let digest = hasher.finalize();
        let hash_hex = hex::encode(digest);

        let final_url = format!(
            "https://{}/get-mp3/{}/{}{}",
            full_info.host, hash_hex, full_info.ts, full_info.path
        );

        Ok(final_url)
    }

    pub async fn download_track_to_file(&self, url: &str, dst_path: &std::path::Path) -> Result<()> {
        use std::io::Write;
        let mut resp = self.http.get(url).send().await?;
        let tmp_path = dst_path.with_extension(format!("tmp.{}", std::process::id()));

        let mut file = std::fs::File::create(&tmp_path)?;
        while let Some(chunk) = resp.chunk().await? {
            file.write_all(&chunk)?;
        }
        file.flush()?;
        std::fs::rename(&tmp_path, dst_path)?;
        Ok(())
    }

    pub async fn download_track_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let bytes = self.http.get(url).send().await?.bytes().await?;
        Ok(bytes.to_vec())
    }

    // Rotor / My Wave
    pub async fn rotor_new_session(&self) -> Result<RotorStationTracks> {
        let url = format!("{}/rotor/session/new", BASE_URL);
        let body = json!({
            "includeTracksInResponse": true,
            "includeWaveModel": false,
            "interactive": true,
            "seeds": ["user:onyourwave"]
        });

        let val: serde_json::Value = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        let res = val.get("result").context("Empty rotor session response")?;
        let station_tracks: RotorStationTracks = serde_json::from_value(res.clone())?;
        Ok(station_tracks)
    }

    pub async fn rotor_next_tracks(&self, session_id: &str, queue: &[String]) -> Result<Vec<Track>> {
        let url = format!("{}/rotor/session/{}/tracks", BASE_URL, session_id);
        let body = json!({
            "feedbacks": [],
            "queue": queue
        });

        let val: serde_json::Value = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        let res = val.get("result").context("Empty rotor next tracks response")?;
        let result: RotorStationTracks = serde_json::from_value(res.clone())?;
        let tracks: Vec<Track> = result.sequence.into_iter().map(|it| it.track).collect();
        Ok(tracks)
    }
}
