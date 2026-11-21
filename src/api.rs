use anyhow::{anyhow, Context, Result};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

pub const API_BUILDS_URL: &str =
    "https://apis.netmarble.com/cpplauncher/api/game/monster2/builds?buildCode=A";

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
}

pub struct ApiClient {
    client: reqwest::Client,
}

impl Default for ApiClient {
    fn default() -> Self {
        Self::new()
    }

    #[test]
    fn resolve_manifest_entry_usa_version_o_default() {
        let manifest: super::BackgroundManifest = serde_json::from_str(r#"{
            "versions": {
                "1.05.00": { "video_url": "https://cdn.example.com/105.mp4", "poster_url": "", "slides": [] },
                "default": { "video_url": "", "poster_url": "https://cdn.example.com/default.jpg", "slides": [] }
            }
        }"#).unwrap();
        assert_eq!(ApiClient::resolve_manifest_entry(&manifest, "1.05.00").video_url, "https://cdn.example.com/105.mp4");
        assert_eq!(ApiClient::resolve_manifest_entry(&manifest, "9.99.00").poster_url, "https://cdn.example.com/default.jpg");
    }

    #[test]
    fn parse_official_site_background_extrae_video_y_poster() {
        let html = r#"poster:"https:\u002F\u002Fsgimage.netmarble.com\u002Fimages\u002Fposter.jpg",bgGB_pc:"https:\u002F\u002Fsgimage.netmarble.com\u002Fimages\u002Fhero.mp4""#;
        let (video, poster) = ApiClient::parse_official_site_background(html).unwrap();
        assert_eq!(video, "https://sgimage.netmarble.com/images/hero.mp4");
        assert_eq!(poster, "https://sgimage.netmarble.com/images/poster.jpg");
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
            .connect_timeout(std::time::Duration::from_secs(15))
            .timeout(std::time::Duration::from_secs(30))
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

        Self::parse_build_info_json(json)
    }

    pub fn parse_build_info_json(json: serde_json::Value) -> Result<BuildInfo> {
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
            .or_else(|| target.get("buildSeq"))
            .or_else(|| target.get("seq"))
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow!("Campo buildSequence faltante en respuesta de Netmarble API"))?;

        // Extract version
        let version = target
            .get("version")
            .or_else(|| target.get("buildVersion"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| anyhow!("Campo version faltante en respuesta de Netmarble API"))?
            .to_string();

        // Extract download URL
        let download_url = target
            .get("buildDownloadUrl")
            .or_else(|| target.get("downloadUrl"))
            .or_else(|| target.get("fileUrl"))
            .or_else(|| target.get("url"))
            .and_then(|v| v.as_str())
            .filter(|s| s.starts_with("https://") || s.starts_with("http://"))
            .ok_or_else(|| anyhow!("Campo downloadUrl inválido o faltante en Netmarble API"))?
            .to_string();

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
            .unwrap_or(0);

        Ok(BuildInfo {
            version,
            build_sequence,
            download_url,
            file_hash,
            file_size,
        })
    }
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundSlide {
    pub tag: String,
    pub title: String,
    pub desc: String,
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackgroundEntry {
    #[serde(default)]
    pub video_url: String,
    #[serde(default)]
    pub poster_url: String,
    #[serde(default)]
    pub slides: Vec<BackgroundSlide>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackgroundManifest {
    #[serde(default)]
    pub versions: std::collections::HashMap<String, BackgroundEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedBackgroundMedia {
    pub version: String,
    pub animated: bool,
    pub video_url: String,
    pub poster_url: String,
    pub cached_video_path: Option<String>,
    pub cached_poster_path: Option<String>,
    pub slides: Vec<BackgroundSlide>,
}

const MAX_MEDIA_BYTES: usize = 50 * 1024 * 1024;

fn sanitize_version_tag(version: &str) -> String {
    let cleaned: String = version
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' })
        .collect();
    if cleaned.is_empty() { "default".to_string() } else { cleaned }
}

fn extension_for_media(url: &str, fallback: &str) -> String {
    let base = url.split('?').next().unwrap_or(url);
    if let Some(ext) = base.rsplit('.').next() {
        let lower = ext.to_ascii_lowercase();
        if matches!(lower.as_str(), "mp4" | "webm" | "jpg" | "jpeg" | "png" | "webp") {
            return lower;
        }
    }
    fallback.to_string()
}

impl ApiClient {
    pub fn resolve_manifest_entry(manifest: &BackgroundManifest, version: &str) -> BackgroundEntry {
        if let Some(entry) = manifest.versions.get(version) {
            return entry.clone();
        }
        manifest.versions.get("default").cloned().unwrap_or_default()
    }

    pub async fn fetch_remote_manifest(&self, url: &str) -> Result<BackgroundManifest> {
        if !url.starts_with("https://") {
            return Err(anyhow!("La URL del manifiesto de fondos debe usar HTTPS"));
        }
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .context("No se pudo descargar el manifiesto de fondos")?;
        if !resp.status().is_success() {
            return Err(anyhow!("Manifiesto de fondos devolvió HTTP {}", resp.status()));
        }
        let manifest: BackgroundManifest = resp
            .json()
            .await
            .context("JSON de manifiesto de fondos inválido")?;
        Ok(manifest)
    }

    pub async fn cache_media_file(
        &self,
        url: &str,
        cache_dir: &std::path::Path,
        version: &str,
        kind: &str,
        fallback_ext: &str,
    ) -> Result<std::path::PathBuf> {
        if !url.starts_with("https://") {
            return Err(anyhow!("Solo se permiten URLs HTTPS para fondos"));
        }
        std::fs::create_dir_all(cache_dir)?;
        let ext = extension_for_media(url, fallback_ext);
        let filename = format!("{}_{}.{}", sanitize_version_tag(version), kind, ext);
        let target = cache_dir.join(filename);
        if target.exists() && std::fs::metadata(&target).map(|m| m.len() > 0).unwrap_or(false) {
            return Ok(target);
        }

        let resp = self.client.get(url).send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("Descarga de medio falló con HTTP {}", resp.status()));
        }
        if let Some(len) = resp.content_length() {
            if len as usize > MAX_MEDIA_BYTES {
                return Err(anyhow!("El fondo excede el límite de 50 MiB"));
            }
        }
        let bytes = resp.bytes().await?;
        if bytes.len() > MAX_MEDIA_BYTES {
            return Err(anyhow!("El fondo excede el límite de 50 MiB"));
        }
        let tmp = target.with_extension(format!("{}.tmp", ext));
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, &target)?;
        Ok(target)
    }
}


pub const OFFICIAL_SITE_URL: &str = "https://stardive.netmarble.com/es";

fn extract_quoted_field(haystack: &str, key: &str) -> Option<String> {
    let needle = format!("{}:\"", key);
    let start = haystack.find(&needle)? + needle.len();
    let rest = &haystack[start..];
    let end = rest.find('"')?;
    let val = &rest[..end];
    if val.starts_with("https://") {
        Some(val.to_string())
    } else {
        None
    }
}

impl ApiClient {
    /// Extrae automáticamente el video de fondo PC global (`bgGB_pc` o `bg_pc`)
    /// y el póster desde el sitio oficial de Netmarble cuando sale un nuevo parche.
    pub fn parse_official_site_background(raw_html: &str) -> Option<(String, String)> {
        let normalized = raw_html.replace("\\u002F", "/");
        let video = extract_quoted_field(&normalized, "bgGB_pc")
            .or_else(|| extract_quoted_field(&normalized, "bg_pc"))
            .filter(|u| u.ends_with(".mp4") || u.ends_with(".webm"))?;

        // Buscar póster cercano antes de bgGB_pc/bg_pc si existe
        let poster = normalized
            .find("bgGB_pc:\"")
            .or_else(|| normalized.find("bg_pc:\""))
            .and_then(|pos| {
                let window_start = pos.saturating_sub(300);
                let slice = &normalized[window_start..pos];
                let jpg_end = slice.rfind(".jpg")? + 4;
                let url_start = slice[..jpg_end].rfind("https://")?;
                Some(slice[url_start..jpg_end].to_string())
            })
            .unwrap_or_default();

        Some((video, poster))
    }

    pub async fn fetch_official_site_background(&self) -> Result<(String, String)> {
        let resp = self
            .client
            .get(OFFICIAL_SITE_URL)
            .send()
            .await
            .context("No se pudo conectar al sitio oficial de Star Dive")?;
        if !resp.status().is_success() {
            return Err(anyhow!("Sitio oficial devolvió HTTP {}", resp.status()));
        }
        let html = resp.text().await?;
        Self::parse_official_site_background(&html)
            .ok_or_else(|| anyhow!("No se encontró video de fondo oficial en el HTML"))
    }
}


fn is_latin_title(title: &str) -> bool {
    let mut ascii_letters = 0usize;
    let mut non_latin = 0usize;
    for ch in title.chars() {
        if ch.is_ascii_alphabetic() {
            ascii_letters += 1;
        } else if ('\u{0E00}'..='\u{0E7F}').contains(&ch)
            || ('\u{4E00}'..='\u{9FFF}').contains(&ch)
            || ('\u{3040}'..='\u{30FF}').contains(&ch)
            || ('\u{AC00}'..='\u{D7AF}').contains(&ch)
        {
            non_latin += 1;
        }
    }
    ascii_letters >= 4 && non_latin == 0
}

fn summarize_forum_content(content: &str) -> String {
    let cleaned = content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("Tame the Unknown") && !l.starts_with("Hello."))
        .collect::<Vec<_>>()
        .join(" ");
    let compact: String = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() > 115 {
        let short: String = compact.chars().take(112).collect();
        format!("{}...", short.trim_end())
    } else if compact.is_empty() {
        "Haz clic para leer el anuncio oficial completo en el foro de Netmarble.".to_string()
    } else {
        compact
    }
}

impl ApiClient {
    async fn fetch_forum_board_latest(
        &self,
        menu_seq: u32,
        tag: &str,
    ) -> Option<BackgroundSlide> {
        let url = format!(
            "https://forum.netmarble.com/api/game/monster2/official/forum/stardive_gl/article/list?rows=9&start=0&menuSeq={}",
            menu_seq
        );
        let referer = format!("https://forum.netmarble.com/stardive_gl/list/{}/1", menu_seq);
        let resp = self
            .client
            .get(&url)
            .header("Referer", referer)
            .header("Origin", "https://forum.netmarble.com")
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let json: serde_json::Value = resp.json().await.ok()?;
        let articles = json.get("articleList")?.as_array()?;
        let chosen = articles.iter().find(|item| {
            item.get("title")
                .and_then(|v| v.as_str())
                .map(is_latin_title)
                .unwrap_or(false)
        })?;
        let id = chosen.get("id")?.as_u64()?;
        let title = chosen.get("title")?.as_str()?.trim().to_string();
        let content = chosen.get("content").and_then(|v| v.as_str()).unwrap_or("");
        Some(BackgroundSlide {
            tag: tag.to_string(),
            title,
            desc: summarize_forum_content(content),
            url: format!("https://forum.netmarble.com/stardive_gl/view/{}/{}", menu_seq, id),
        })
    }

    pub async fn fetch_official_forum_slides(&self) -> Vec<BackgroundSlide> {
        let (update, event, notice) = tokio::join!(
            self.fetch_forum_board_latest(8, "ACTUALIZACIÓN"),
            self.fetch_forum_board_latest(6, "EVENTO"),
            self.fetch_forum_board_latest(2, "AVISO"),
        );
        [update, event, notice].into_iter().flatten().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::ApiClient;

    #[test]
    fn parse_build_info_rechaza_payload_incompleto_sin_fallback_silencioso() {
        let incomplete = serde_json::json!({ "data": {} });
        assert!(ApiClient::parse_build_info_json(incomplete).is_err());
    }

    #[test]
    fn parse_build_info_acepta_payload_valido() {
        let valid = serde_json::json!({
            "data": [{
                "buildSequence": 72,
                "version": "1.04.00",
                "buildDownloadUrl": "https://cdn.example.com/client.zip",
                "buildFileHash": "abc123",
                "buildFileSize": "123456"
            }]
        });
        let info = ApiClient::parse_build_info_json(valid).unwrap();
        assert_eq!(info.build_sequence, 72);
        assert_eq!(info.version, "1.04.00");
        assert_eq!(info.download_url, "https://cdn.example.com/client.zip");
        assert_eq!(info.file_size, 123456);
    }
}
