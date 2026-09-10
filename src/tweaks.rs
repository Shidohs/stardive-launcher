#![allow(dead_code)]
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

pub struct SystemTweaks;

impl SystemTweaks {
    /// Detect all installed Proton / Wine runners on the system
    pub fn detect_proton_runners() -> Vec<PathBuf> {
        let mut runners = Vec::new();
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/shidox"));

        let search_dirs = [
            home.join(".local")
                .join("share")
                .join("Steam")
                .join("compatibilitytools.d"),
            home.join(".steam")
                .join("root")
                .join("compatibilitytools.d"),
            home.join(".steam")
                .join("steam")
                .join("compatibilitytools.d"),
            home.join(".local")
                .join("share")
                .join("lutris")
                .join("runners")
                .join("wine"),
            PathBuf::from("/usr/share/steam/compatibilitytools.d"),
        ];

        for dir in &search_dirs {
            if dir.exists() {
                if let Ok(entries) = fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() {
                            // Check if it has a proton executable or wine executable
                            if path.join("proton").exists()
                                || path.join("files").join("bin").join("wine").exists()
                                || path.join("bin").join("wine").exists()
                            {
                                runners.push(path);
                            }
                        }
                    }
                }
            }
        }

        runners.sort();
        runners.dedup();
        runners
    }

    /// Check if gamemoderun is installed
    pub fn has_gamemode() -> bool {
        which_exists("gamemoderun")
    }

    /// Check if mangohud is installed
    pub fn has_mangohud() -> bool {
        which_exists("mangohud")
    }

    /// Check if gamescope is installed
    pub fn has_gamescope() -> bool {
        which_exists("gamescope")
    }

    /// Apply Unreal Engine 5 FPS Unlocker tweaks to Mongil: Star Dive
    pub fn apply_fps_tweak(prefix_dir: &Path, fps_limit: u32, vsync: bool) -> Result<()> {
        // Look in drive_c/users/<user>/AppData/Local/BigCat/Saved/Config/WindowsNoEditor/
        let users_dir = prefix_dir.join("drive_c").join("users");
        if !users_dir.exists() {
            return Ok(());
        }

        let entries = fs::read_dir(&users_dir)?;
        for entry in entries.flatten() {
            let user_path = entry.path();
            if user_path.is_dir() {
                let config_dir = user_path
                    .join("AppData")
                    .join("Local")
                    .join("BigCat")
                    .join("Saved")
                    .join("Config")
                    .join("WindowsNoEditor");

                if !config_dir.exists() {
                    let _ = fs::create_dir_all(&config_dir);
                }

                // Tweak Engine.ini
                let engine_ini = config_dir.join("Engine.ini");
                let mut content = if engine_ini.exists() {
                    fs::read_to_string(&engine_ini).unwrap_or_default()
                } else {
                    String::new()
                };

                let tweak_block = format!(
                    "\n[/Script/Engine.Engine]\nbSmoothFrameRate=False\nMinSmoothedFrameRate=0\nMaxSmoothedFrameRate={}\n\n[/Script/Engine.UserInterfaceSettings]\nApplicationScale=1.0\n",
                    if fps_limit == 0 { 240 } else { fps_limit }
                );

                if !content.contains("[/Script/Engine.Engine]") {
                    content.push_str(&tweak_block);
                    let _ = fs::write(&engine_ini, content);
                }

                // Tweak GameUserSettings.ini
                let gus_ini = config_dir.join("GameUserSettings.ini");
                let mut gus_content = if gus_ini.exists() {
                    fs::read_to_string(&gus_ini).unwrap_or_default()
                } else {
                    String::new()
                };

                let fps_val = if fps_limit == 0 {
                    "0.000000"
                } else {
                    &format!("{}.000000", fps_limit)
                };
                let vsync_val = if vsync { "True" } else { "False" };

                if !gus_content.contains("[/Script/BigCat.BigCatGameUserSettings]")
                    && !gus_content.contains("[/Script/Engine.GameUserSettings]")
                {
                    gus_content.push_str(&format!(
                        "\n[/Script/Engine.GameUserSettings]\nFrameRateLimit={}\nbUseVSync={}\n",
                        fps_val, vsync_val
                    ));
                } else {
                    // Update existing FrameRateLimit and bUseVSync
                    let mut lines: Vec<String> =
                        gus_content.lines().map(|s| s.to_string()).collect();
                    let mut found_fps = false;
                    let mut found_vsync = false;

                    for line in &mut lines {
                        if line.trim_start().starts_with("FrameRateLimit=") {
                            *line = format!("FrameRateLimit={}", fps_val);
                            found_fps = true;
                        }
                        if line.trim_start().starts_with("bUseVSync=") {
                            *line = format!("bUseVSync={}", vsync_val);
                            found_vsync = true;
                        }
                    }

                    if !found_fps {
                        lines.push(format!("FrameRateLimit={}", fps_val));
                    }
                    if !found_vsync {
                        lines.push(format!("bUseVSync={}", vsync_val));
                    }
                    gus_content = lines.join("\n");
                }

                let _ = fs::write(&gus_ini, gus_content);
            }
        }

        Ok(())
    }

    /// Clear DXVK and VKD3D shader cache to resolve stuttering after updates
    pub fn clear_shader_cache(prefix_dir: &Path) -> Result<usize> {
        let mut count = 0;
        let cache_dirs = [
            prefix_dir.join("shadercache"),
            prefix_dir
                .join("drive_c")
                .join("Program Files")
                .join("Netmarble")
                .join("Netmarble Game")
                .join("STARDIVE"),
        ];

        for dir in &cache_dirs {
            if dir.exists() {
                if let Ok(entries) = fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                            if ext == "dxvk-cache" || ext == "vkd3d-cache" || ext == "cache" {
                                if fs::remove_file(&path).is_ok() {
                                    count += 1;
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(count)
    }
}

fn which_exists(bin: &str) -> bool {
    if let Ok(path_var) = std::env::var("PATH") {
        for p in path_var.split(':') {
            let candidate = Path::new(p).join(bin);
            if candidate.exists() && candidate.is_file() {
                return true;
            }
        }
    }
    false
}
