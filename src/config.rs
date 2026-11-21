use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Default, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum RenderApi {
    #[default]
    DirectX12, // Usa VKD3D-Proton (Recomendado para UE5)
    DirectX11, // Usa DXVK / DXVK-Async
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LauncherConfig {
    pub game_dir: PathBuf,
    pub prefix_dir: PathBuf,
    pub proton_path: PathBuf,
    pub executable: String,
    pub launch_args: String,
    pub dxvk_async: bool,
    pub wine_esync: bool,
    pub wine_fsync: bool,
    pub nvapi: bool,
    #[serde(default)]
    pub proton_use_wined3d: bool,
    #[serde(default)]
    pub use_gamemode: bool,
    #[serde(default)]
    pub use_mangohud: bool,
    #[serde(default)]
    pub use_gamescope: bool,
    #[serde(default = "default_gamescope_args")]
    pub gamescope_args: String,
    #[serde(default = "default_fps_limit")]
    pub fps_limit: u32,
    #[serde(default)]
    pub vsync: bool,
    #[serde(default)]
    pub render_api: Option<RenderApi>,
    #[serde(default)]
    pub use_umu: bool,
    #[serde(default)]
    pub enable_nvapi_dlss: bool,
    #[serde(default)]
    pub enable_wayland: bool,
    #[serde(default = "default_close_action")]
    pub close_action: String,
    #[serde(default)]
    pub disable_motion_blur: bool,
    #[serde(default)]
    pub disable_chromatic_aberration: bool,
    #[serde(default)]
    pub disable_film_grain: bool,
    #[serde(default = "default_animated_background")]
    pub animated_background: bool,
    #[serde(default)]
    pub background_manifest_url: String,
    #[serde(default)]
    pub custom_env: HashMap<String, String>,
}

fn default_animated_background() -> bool {
    true
}

fn default_gamescope_args() -> String {
    "-W 1920 -H 1080 -f".to_string()
}

fn default_fps_limit() -> u32 {
    144
}

fn default_close_action() -> String {
    "minimize".to_string()
}

impl Default for LauncherConfig {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let default_prefix = home.join("Games").join("mongil-star-dive");
        let default_game_dir = default_prefix
            .join("drive_c")
            .join("Program Files")
            .join("Netmarble")
            .join("Netmarble Game")
            .join("STARDIVE");
        let default_proton = home
            .join(".local")
            .join("share")
            .join("Steam")
            .join("compatibilitytools.d")
            .join("GE-Proton11-6-x86_64");

        Self {
            game_dir: default_game_dir,
            prefix_dir: default_prefix,
            proton_path: default_proton,
            executable: "STARDIVE.exe".to_string(),
            launch_args: "NMENV=nmp".to_string(),
            dxvk_async: true,
            wine_esync: true,
            wine_fsync: true,
            nvapi: true,
            proton_use_wined3d: false,
            use_gamemode: true,
            use_mangohud: false,
            use_gamescope: false,
            gamescope_args: default_gamescope_args(),
            fps_limit: default_fps_limit(),
            vsync: false,
            render_api: Some(RenderApi::DirectX12),
            use_umu: false,
            enable_nvapi_dlss: true,
            enable_wayland: false,
            close_action: default_close_action(),
            disable_motion_blur: false,
            disable_chromatic_aberration: false,
            disable_film_grain: false,
            animated_background: default_animated_background(),
            background_manifest_url: String::new(),
            custom_env: HashMap::new(),
        }
    }
}

impl LauncherConfig {
    pub fn app_config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".config")
            })
            .join("stardive-launcher")
    }

    pub fn config_file_path() -> PathBuf {
        Self::app_config_dir().join("config.json")
    }

    pub fn app_media_cache_dir() -> PathBuf {
        dirs::cache_dir()
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".cache")
            })
            .join("stardive-launcher")
            .join("media")
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_file_path();
        Self::load_from_path(&path)
    }

    pub fn load_from_path(path: &std::path::Path) -> Result<Self> {
        if path.exists() {
            let data = fs::read_to_string(path)
                .with_context(|| format!("Failed to read config from {:?}", path))?;
            match serde_json::from_str::<LauncherConfig>(&data) {
                Ok(cfg) => return Ok(cfg),
                Err(err) => {
                    let backup_path = path.with_extension("json.bak");
                    if let Err(backup_err) = fs::copy(path, &backup_path) {
                        eprintln!(
                            "Warning: failed to backup corrupt config.json to {:?}: {}",
                            backup_path, backup_err
                        );
                    } else {
                        eprintln!(
                            "Warning: parsing config.json failed ({}). Backup saved at {:?}. Falling back to default.",
                            err, backup_path
                        );
                    }
                }
            }
        }
        let default_cfg = Self::default();
        let _ = default_cfg.save_to_path(path);
        Ok(default_cfg)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_file_path();
        self.save_to_path(&path)
    }

    pub fn save_to_path(&self, path: &std::path::Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config dir {:?}", parent))?;
        }
        let json =
            serde_json::to_string_pretty(self).context("Failed to serialize config to JSON")?;
        fs::write(path, json).with_context(|| format!("Failed to write config to {:?}", path))?;
        Ok(())
    }

    pub fn is_installed(&self) -> bool {
        self.game_exe_path().exists()
    }

    pub fn game_exe_path(&self) -> PathBuf {
        self.game_dir.join(&self.executable)
    }

    pub fn secondary_exe_path(&self) -> PathBuf {
        self.game_dir
            .join("BigCat")
            .join("Binaries")
            .join("Win64")
            .join("BigCat-Win64-Shipping.exe")
    }

    pub fn proton_bin(&self) -> PathBuf {
        self.proton_path.join("proton")
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.game_dir.join("game_manifest.json")
    }

    pub fn log_file_path(&self) -> PathBuf {
        self.prefix_dir.join("launcher_game.log")
    }
}

#[cfg(test)]
mod tests {
    use super::LauncherConfig;
    use std::fs;

    #[test]
    fn load_from_path_respalda_config_corrupta_antes_de_regenerar() {
        let base = std::env::temp_dir().join(format!("stardive_cfg_test_{}", std::process::id()));
        fs::create_dir_all(&base).unwrap();
        let cfg_path = base.join("config.json");
        let corrupt = "{ invalid json ::: ";
        fs::write(&cfg_path, corrupt).unwrap();

        let loaded = LauncherConfig::load_from_path(&cfg_path).unwrap();
        assert_eq!(loaded.executable, "STARDIVE.exe");

        let backup_path = cfg_path.with_extension("json.bak");
        assert!(backup_path.exists(), "Debe crear respaldo .json.bak");
        assert_eq!(fs::read_to_string(&backup_path).unwrap(), corrupt);

        let _ = fs::remove_dir_all(&base);
    }
}
