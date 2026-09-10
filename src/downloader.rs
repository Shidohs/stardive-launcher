use anyhow::{anyhow, Context, Result};
use md5::{Digest, Md5};
use reqwest::header::RANGE;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc::Sender;

#[derive(Debug, Clone)]
pub struct DownloadProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub speed_bytes_per_sec: f64,
    pub progress_ratio: f32,
    pub eta_seconds: Option<u64>,
    pub status: String,
    pub is_completed: bool,
}

impl DownloadProgress {
    pub fn speed_formatted(&self) -> String {
        let mb = self.speed_bytes_per_sec / (1024.0 * 1024.0);
        if mb >= 1.0 {
            format!("{:.2} MB/s", mb)
        } else {
            let kb = self.speed_bytes_per_sec / 1024.0;
            format!("{:.1} KB/s", kb)
        }
    }

    pub fn downloaded_mb(&self) -> f64 {
        self.downloaded_bytes as f64 / (1024.0 * 1024.0)
    }

    pub fn total_mb(&self) -> f64 {
        self.total_bytes as f64 / (1024.0 * 1024.0)
    }

    pub fn eta_formatted(&self) -> String {
        match self.eta_seconds {
            Some(secs) => {
                if secs >= 3600 {
                    format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
                } else if secs >= 60 {
                    format!("{}m {}s", secs / 60, secs % 60)
                } else {
                    format!("{}s", secs)
                }
            }
            None => "--".to_string(),
        }
    }
}

pub struct Downloader;

impl Downloader {
    pub async fn download(
        url: &str,
        dest_file: &Path,
        expected_hash: &str,
        progress_tx: Sender<DownloadProgress>,
        cancel_token: Arc<AtomicBool>,
    ) -> Result<PathBuf> {
        let part_file = dest_file.with_extension("zip.part");

        if let Some(parent) = dest_file.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create download folder {:?}", parent))?;
        }

        // Check if partial download exists
        let mut existing_bytes = 0u64;
        if part_file.exists() {
            if let Ok(metadata) = fs::metadata(&part_file) {
                existing_bytes = metadata.len();
            }
        }

        let client = reqwest::Client::builder()
            .user_agent("NetmarbleLauncher/1.0.0")
            .build()?;

        let mut req = client.get(url);
        if existing_bytes > 0 {
            req = req.header(RANGE, format!("bytes={}-", existing_bytes));
        }

        let mut resp = req
            .send()
            .await
            .context("Failed to start download stream")?;
        let status = resp.status();

        let (mut file, mut current_bytes, total_bytes) =
            if status == reqwest::StatusCode::PARTIAL_CONTENT {
                let content_len = resp.content_length().unwrap_or(0);
                let total = existing_bytes + content_len;
                let file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&part_file)
                    .context("Failed to open part file for appending")?;
                (file, existing_bytes, total)
            } else if status.is_success() {
                let total = resp.content_length().unwrap_or(0);
                let file = OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(&part_file)
                    .context("Failed to create part file")?;
                (file, 0u64, total)
            } else {
                return Err(anyhow!("HTTP error during download: {}", status));
            };

        let mut last_update_time = Instant::now();
        let mut bytes_since_last_update = 0u64;

        while let Some(chunk) = resp.chunk().await.context("Error reading response chunk")? {
            if cancel_token.load(Ordering::Relaxed) {
                let _ = progress_tx
                    .send(DownloadProgress {
                        downloaded_bytes: current_bytes,
                        total_bytes,
                        speed_bytes_per_sec: 0.0,
                        progress_ratio: if total_bytes > 0 {
                            (current_bytes as f32) / (total_bytes as f32)
                        } else {
                            0.0
                        },
                        eta_seconds: None,
                        status: "Descarga pausada/cancelada".to_string(),
                        is_completed: false,
                    })
                    .await;
                return Err(anyhow!("Descarga cancelada por el usuario"));
            }

            file.write_all(&chunk)
                .context("Failed to write chunk to disk")?;

            let chunk_len = chunk.len() as u64;
            current_bytes += chunk_len;
            bytes_since_last_update += chunk_len;

            let now = Instant::now();
            let elapsed_since_update = now.duration_since(last_update_time).as_secs_f64();

            if elapsed_since_update >= 0.25 || current_bytes >= total_bytes {
                let speed = bytes_since_last_update as f64 / elapsed_since_update.max(0.001);
                last_update_time = now;
                bytes_since_last_update = 0;

                let progress_ratio = if total_bytes > 0 {
                    ((current_bytes as f32) / (total_bytes as f32)).min(1.0)
                } else {
                    0.0
                };

                let remaining_bytes = total_bytes.saturating_sub(current_bytes);
                let eta_seconds = if speed > 1024.0 {
                    Some((remaining_bytes as f64 / speed) as u64)
                } else {
                    None
                };

                let _ = progress_tx
                    .send(DownloadProgress {
                        downloaded_bytes: current_bytes,
                        total_bytes,
                        speed_bytes_per_sec: speed,
                        progress_ratio,
                        eta_seconds,
                        status: "Descargando cliente del juego...".to_string(),
                        is_completed: false,
                    })
                    .await;
            }
        }

        file.flush().context("Failed to flush download file")?;
        drop(file);

        // Verification MD5
        if !expected_hash.is_empty() {
            let _ = progress_tx
                .send(DownloadProgress {
                    downloaded_bytes: current_bytes,
                    total_bytes,
                    speed_bytes_per_sec: 0.0,
                    progress_ratio: 1.0,
                    eta_seconds: Some(0),
                    status: "Verificando integridad MD5...".to_string(),
                    is_completed: false,
                })
                .await;

            let computed_hash = Self::calculate_md5(&part_file)?;
            if !computed_hash.eq_ignore_ascii_case(expected_hash) {
                eprintln!(
                    "Advertencia: Hash MD5 calculado ({}) difiere del esperado ({})",
                    computed_hash, expected_hash
                );
            }
        }

        // Rename part to target file
        if dest_file.exists() {
            let _ = fs::remove_file(dest_file);
        }
        fs::rename(&part_file, dest_file)
            .context("Failed to rename temporary download file to destination")?;

        let _ = progress_tx
            .send(DownloadProgress {
                downloaded_bytes: total_bytes,
                total_bytes,
                speed_bytes_per_sec: 0.0,
                progress_ratio: 1.0,
                eta_seconds: Some(0),
                status: "Descarga completada con éxito".to_string(),
                is_completed: true,
            })
            .await;

        Ok(dest_file.to_path_buf())
    }

    pub fn calculate_md5(file_path: &Path) -> Result<String> {
        let mut file = File::open(file_path)
            .with_context(|| format!("Failed to open {:?} for checksum", file_path))?;
        let mut hasher = Md5::new();
        let mut buffer = [0u8; 1024 * 1024]; // 1MB buffer

        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }

        let hash = hasher.finalize();
        Ok(hex::encode(hash))
    }
}
