use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

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
    pub custom_env: HashMap<String, String>,
}

fn default_gamescope_args() -> String {
    "-W 1920 -H 1080 -f".to_string()
}

fn default_fps_limit() -> u32 {
    144
}

impl Default for LauncherConfig {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/shidox"));
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
            custom_env: HashMap::new(),
        }
    }
}

impl LauncherConfig {
    pub fn config_file_path() -> PathBuf {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("/home/shidox"))
                    .join(".config")
            })
            .join("stardive-launcher");
        config_dir.join("config.json")
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_file_path();
        if path.exists() {
            let data = fs::read_to_string(&path)
                .with_context(|| format!("Failed to read config from {:?}", path))?;
            match serde_json::from_str::<LauncherConfig>(&data) {
                Ok(cfg) => return Ok(cfg),
                Err(err) => {
                    eprintln!(
                        "Warning: parsing config.json failed ({}). Falling back to default.",
                        err
                    );
                }
            }
        }
        let default_cfg = Self::default();
        let _ = default_cfg.save();
        Ok(default_cfg)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_file_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config dir {:?}", parent))?;
        }
        let json =
            serde_json::to_string_pretty(self).context("Failed to serialize config to JSON")?;
        fs::write(&path, json).with_context(|| format!("Failed to write config to {:?}", path))?;
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
