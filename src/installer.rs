use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameManifest {
    pub version: String,
    pub build_sequence: u64,
    pub installed_at: String,
    pub executable: String,
    pub secondary_executable: String,
}

pub struct Installer;

impl Installer {
    pub fn read_manifest(game_dir: &Path) -> Option<GameManifest> {
        let manifest_path = game_dir.join("game_manifest.json");
        if !manifest_path.exists() {
            return None;
        }
        let content = fs::read_to_string(&manifest_path).ok()?;
        serde_json::from_str(&content).ok()
    }

    pub fn write_manifest(game_dir: &Path, version: &str, build_sequence: u64) -> Result<()> {
        let manifest = GameManifest {
            version: version.to_string(),
            build_sequence,
            installed_at: Utc::now().to_rfc3339(),
            executable: "STARDIVE.exe".to_string(),
            secondary_executable: "BigCat/Binaries/Win64/BigCat-Win64-Shipping.exe".to_string(),
        };

        let json =
            serde_json::to_string_pretty(&manifest).context("Failed to serialize game manifest")?;
        let manifest_path = game_dir.join("game_manifest.json");
        fs::write(&manifest_path, json)
            .with_context(|| format!("Failed to write manifest to {:?}", manifest_path))?;
        Ok(())
    }

    pub fn extract_zip(
        zip_path: &Path,
        dest_dir: &Path,
        version: &str,
        build_sequence: u64,
        cancel_token: Option<Arc<AtomicBool>>,
    ) -> Result<()> {
        fs::create_dir_all(dest_dir)
            .with_context(|| format!("Failed to create destination directory {:?}", dest_dir))?;

        let file = File::open(zip_path)
            .with_context(|| format!("Failed to open zip archive {:?}", zip_path))?;
        let mut archive = zip::ZipArchive::new(file)
            .with_context(|| format!("Failed to read zip archive {:?}", zip_path))?;

        let total_files = archive.len();

        for i in 0..total_files {
            if let Some(ref token) = cancel_token {
                if token.load(Ordering::Relaxed) {
                    return Err(anyhow!("Instalación cancelada por el usuario"));
                }
            }

            let mut zip_file = archive
                .by_index(i)
                .with_context(|| format!("Failed to read entry {} from zip", i))?;

            let enclosed = match zip_file.enclosed_name() {
                Some(p) => p.to_path_buf(),
                None => continue,
            };

            let outpath = dest_dir.join(&enclosed);

            if (*zip_file.name()).ends_with('/') {
                fs::create_dir_all(&outpath)
                    .with_context(|| format!("Failed to create directory {:?}", outpath))?;
            } else {
                if let Some(parent) = outpath.parent() {
                    if !parent.exists() {
                        fs::create_dir_all(parent)
                            .with_context(|| format!("Failed to create parent dir {:?}", parent))?;
                    }
                }

                let mut outfile = File::create(&outpath)
                    .with_context(|| format!("Failed to create file {:?}", outpath))?;
                io::copy(&mut zip_file, &mut outfile)
                    .with_context(|| format!("Failed to extract content to {:?}", outpath))?;
            }

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Some(mode) = zip_file.unix_mode() {
                    let _ = fs::set_permissions(&outpath, fs::Permissions::from_mode(mode));
                }
            }
        }

        // Save manifest
        Self::write_manifest(dest_dir, version, build_sequence)?;

        // Check executables
        let main_exe = dest_dir.join("STARDIVE.exe");
        if !main_exe.exists() {
            eprintln!(
                "Advertencia: STARDIVE.exe no fue encontrado en {:?}",
                dest_dir
            );
        }

        Ok(())
    }
}
