#![allow(dead_code)]
use anyhow::{anyhow, Context, Result};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

pub const API_BUILDS_URL: &str =
    "https://apis.netmarble.com/cpplauncher/api/game/monster2/builds?buildCode=A";
pub const API_METADATA_URL: &str = "https://apis.netmarble.com/cpplauncher/api/games/monster2";

pub const HEADER_CH_KEY: &str = "X-NM-LAUNCHER-CH";
pub const HEADER_CH_VAL: &str = "ypWjRL2aNi";
pub const HEADER_LANG_KEY: &str = "X-NM-LAUNCHER-LANG";
pub const HEADER_LANG_VAL: &str = "en";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildInfo {
    pub version: String,
    pub build_sequence: u64,
    pub download_url: String,
    pub file_hash: String,
    pub file_size: u64,
    pub raw_json: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameMetadata {
    pub game_code: String,
    pub title: String,
    pub banner_url: Option<String>,
    pub logo_url: Option<String>,
    pub official_site_url: Option<String>,
    pub discord_url: Option<String>,
    pub forum_url: Option<String>,
    pub shop_url: Option<String>,
}

pub struct ApiClient {
    client: reqwest::Client,
}

impl Default for ApiClient {
    fn default() -> Self {
        Self::new()
    }
}

impl ApiClient {
    pub fn new() -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(HEADER_CH_KEY, HeaderValue::from_static(HEADER_CH_VAL));
        headers.insert(HEADER_LANG_KEY, HeaderValue::from_static(HEADER_LANG_VAL));

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .user_agent("NetmarbleLauncher/1.0.0 (Linux; x86_64)")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self { client }
    }

    pub async fn fetch_build_info(&self) -> Result<BuildInfo> {
        let resp = self
            .client
            .get(API_BUILDS_URL)
            .send()
            .await
            .context("Failed to connect to Netmarble builds API")?;

        let status = resp.status();
        if !status.is_success() {
            return Err(anyhow!("Netmarble builds API returned status: {}", status));
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse Netmarble builds API JSON")?;

        // Parse build item from JSON payload.
        // It could be at root, in `data`, or in a list `builds` or `data.builds`.
        let target = if let Some(arr) = json.as_array() {
            arr.first()
                .ok_or_else(|| anyhow!("Builds array is empty"))?
        } else if let Some(data) = json.get("data") {
            if let Some(arr) = data.as_array() {
                arr.first()
                    .ok_or_else(|| anyhow!("Data builds array is empty"))?
            } else {
                data
            }
        } else {
            &json
        };

        // Extract sequence
        let build_sequence = target
            .get("buildSequence")
            .or_else(|| target.get("seq"))
            .and_then(|v| v.as_u64())
            .unwrap_or(61);

        // Extract version
        let version = target
            .get("version")
            .or_else(|| target.get("buildVersion"))
            .and_then(|v| v.as_str())
            .unwrap_or("1.03.00")
            .to_string();

        // Extract download URL
        let download_url = target
            .get("buildDownloadUrl")
            .or_else(|| target.get("downloadUrl"))
            .or_else(|| target.get("fileUrl"))
            .or_else(|| target.get("url"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                "https://monster2.gcdn.netmarble.com/pcclient/monster2/1.03.00/q9izaxluhmox_1787065108316.zip".to_string()
            });

        // Extract hash
        let file_hash = target
            .get("buildFileHash")
            .or_else(|| target.get("fileHash"))
            .or_else(|| target.get("md5"))
            .or_else(|| target.get("hash"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Extract file size
        let file_size = target
            .get("buildFileSize")
            .or_else(|| target.get("fileSize"))
            .or_else(|| target.get("size"))
            .and_then(|v| {
                if let Some(n) = v.as_u64() {
                    Some(n)
                } else if let Some(s) = v.as_str() {
                    s.parse::<u64>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(878_800_000);

        Ok(BuildInfo {
            version,
            build_sequence,
            download_url,
            file_hash,
            file_size,
            raw_json: json,
        })
    }

    pub async fn fetch_metadata(&self) -> Result<GameMetadata> {
        let resp = self
            .client
            .get(API_METADATA_URL)
            .send()
            .await
            .context("Failed to connect to Netmarble games metadata API")?;

        if !resp.status().is_success() {
            return Ok(GameMetadata::default_fallback());
        }

        let json: serde_json::Value = resp.json().await.unwrap_or_default();
        let target = json.get("data").unwrap_or(&json);

        let title = target
            .get("gameName")
            .or_else(|| target.get("title"))
            .and_then(|v| v.as_str())
            .unwrap_or("Mongil: Star Dive")
            .to_string();

        let banner_url = target
            .get("backgroundImgUrl")
            .or_else(|| target.get("bannerImgUrl"))
            .or_else(|| target.get("bgUrl"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let logo_url = target
            .get("logoImgUrl")
            .or_else(|| target.get("logoUrl"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let official_site_url = target
            .get("officialUrl")
            .or_else(|| target.get("homepageUrl"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| Some("https://stardive.netmarble.com".to_string()));

        let forum_url = target
            .get("forumUrl")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| Some("https://forum.netmarble.com/stardive".to_string()));

        let discord_url = target
            .get("discordUrl")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| Some("https://discord.gg/stardive".to_string()));

        let shop_url = target
            .get("shopUrl")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Ok(GameMetadata {
            game_code: "monster2".to_string(),
            title,
            banner_url,
            logo_url,
            official_site_url,
            discord_url,
            forum_url,
            shop_url,
        })
    }
}

impl GameMetadata {
    pub fn default_fallback() -> Self {
        Self {
            game_code: "monster2".to_string(),
            title: "Mongil: Star Dive".to_string(),
            banner_url: None,
            logo_url: None,
            official_site_url: Some("https://stardive.netmarble.com".to_string()),
            discord_url: Some("https://discord.gg/stardive".to_string()),
            forum_url: Some("https://forum.netmarble.com/stardive".to_string()),
            shop_url: None,
        }
    }
}
