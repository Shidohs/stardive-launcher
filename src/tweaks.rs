use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const TWEAKS_MARKER_START: &str = "; --- STARDIVE LAUNCHER TWEAKS START ---";
pub const TWEAKS_MARKER_END: &str = "; --- STARDIVE LAUNCHER TWEAKS END ---";

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Ue5Tweaks {
    #[serde(default)]
    pub max_fps: u32,
    #[serde(default)]
    pub vsync: bool,
    #[serde(default)]
    pub disable_motion_blur: bool,
    #[serde(default)]
    pub disable_chromatic_aberration: bool,
    #[serde(default)]
    pub disable_film_grain: bool,
}

pub struct SystemTweaks;

impl SystemTweaks {
    /// Detect all installed Proton / Wine runners on the system
    pub fn detect_proton_runners() -> Vec<PathBuf> {
        let mut runners = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));

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
            let canonical_dir = fs::canonicalize(dir).unwrap_or_else(|_| dir.clone());
            if canonical_dir.exists() {
                if let Ok(entries) = fs::read_dir(&canonical_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        let canonical_path =
                            fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                        if canonical_path.is_dir() {
                            // Check if it has a proton executable or wine executable
                            if (canonical_path.join("proton").exists()
                                || canonical_path
                                    .join("files")
                                    .join("bin")
                                    .join("wine")
                                    .exists()
                                || canonical_path.join("bin").join("wine").exists())
                                && seen.insert(canonical_path.clone())
                            {
                                runners.push(canonical_path);
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

    /// Check if gamescope is installed
    pub fn has_gamescope() -> bool {
        which_exists("gamescope")
    }

    /// Check if running inside gamescope or Steam Deck
    pub fn is_running_in_gamescope() -> bool {
        std::env::var("GAMESCOPE_WAYLAND_DISPLAY").is_ok()
            || std::env::var("SteamDeck").as_deref() == Ok("1")
    }

    /// Parchea un archivo Engine.ini de Unreal Engine 5 con optimizaciones gráficas y FPS unlocker
    pub fn apply_ue5_tweaks(engine_ini_path: &Path, tweaks: &Ue5Tweaks) -> Result<()> {
        if let Some(parent) = engine_ini_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut content = if engine_ini_path.exists() {
            fs::read_to_string(engine_ini_path).unwrap_or_default()
        } else {
            String::new()
        };

        // Limpiar bloque previo generado por el launcher si existe
        if let (Some(start), Some(end)) = (
            content.find(TWEAKS_MARKER_START),
            content.find(TWEAKS_MARKER_END),
        ) {
            content.replace_range(start..(end + TWEAKS_MARKER_END.len()), "");
        }

        let mut block = format!("\n{}\n[/Script/Engine.Engine]\n", TWEAKS_MARKER_START);
        if tweaks.max_fps > 0 {
            block.push_str(&format!(
                "bSmoothFrameRate=False\nMinDesiredFrameRate=30.000000\nFixedFrameRate={}.000000\nMinSmoothedFrameRate=0\nMaxSmoothedFrameRate={}\n",
                tweaks.max_fps, tweaks.max_fps
            ));
        } else {
            block.push_str(
                "bSmoothFrameRate=False\nMinSmoothedFrameRate=0\nMaxSmoothedFrameRate=240\n",
            );
        }
        block.push_str("\n[/Script/Engine.UserInterfaceSettings]\nApplicationScale=1.0\n");

        block.push_str("\n[SystemSettings]\n");
        if tweaks.max_fps > 0 {
            block.push_str(&format!("t.MaxFPS={}\n", tweaks.max_fps));
        }
        if tweaks.disable_motion_blur {
            block.push_str("r.MotionBlurQuality=0\n");
        }
        if tweaks.disable_chromatic_aberration {
            block.push_str("r.SceneColorFringeQuality=0\nr.SceneColorFringe.Max=0\n");
        }
        if tweaks.disable_film_grain {
            block.push_str("r.Tonemapper.GrainQuantization=0\nr.FilmGrain=0\n");
        }
        block.push_str(&format!("{}\n", TWEAKS_MARKER_END));

        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(engine_ini_path)?;

        file.write_all(content.trim().as_bytes())?;
        file.write_all(block.as_bytes())?;

        Ok(())
    }

    /// Aplica ajustes de UE5 a todos los usuarios configurados en el prefijo de Wine/Proton
    pub fn apply_user_tweaks(prefix_dir: &Path, tweaks: &Ue5Tweaks) -> Result<()> {
        let user_roots = [
            prefix_dir.join("pfx").join("drive_c").join("users"),
            prefix_dir.join("drive_c").join("users"),
        ];
        let Some(users_dir) = user_roots.into_iter().find(|p| p.exists()) else {
            return Ok(());
        };

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

                // 1. Tweak Engine.ini con marcadores limpios
                let engine_ini = config_dir.join("Engine.ini");
                let _ = Self::apply_ue5_tweaks(&engine_ini, tweaks);

                // 2. Tweak GameUserSettings.ini (FrameRateLimit y bUseVSync)
                let gus_ini = config_dir.join("GameUserSettings.ini");
                let gus_content = if gus_ini.exists() {
                    fs::read_to_string(&gus_ini).unwrap_or_default()
                } else {
                    String::new()
                };

                let updated_gus =
                    Self::patch_game_user_settings_ini(&gus_content, tweaks.max_fps, tweaks.vsync);
                let _ = fs::write(&gus_ini, updated_gus);
            }
        }

        Ok(())
    }

    pub fn patch_game_user_settings_ini(content: &str, max_fps: u32, vsync: bool) -> String {
        let fps_val = if max_fps == 0 {
            "0.000000".to_string()
        } else {
            format!("{}.000000", max_fps)
        };
        let vsync_val = if vsync { "True" } else { "False" };

        let target_sections = [
            "[/Script/BigCat.BigCatGameUserSettings]",
            "[/Script/Engine.GameUserSettings]",
        ];

        let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        let section_idx = lines
            .iter()
            .position(|l| target_sections.contains(&l.trim()));

        let Some(sec_start) = section_idx else {
            let mut out = content.to_string();
            out.push_str(&format!(
                "\n[/Script/Engine.GameUserSettings]\nFrameRateLimit={}\nbUseVSync={}\n",
                fps_val, vsync_val
            ));
            return out;
        };

        let sec_end = lines[sec_start + 1..]
            .iter()
            .position(|l| l.trim_start().starts_with('['))
            .map(|o| sec_start + 1 + o)
            .unwrap_or(lines.len());

        let mut found_fps = false;
        let mut found_vsync = false;

        for line in &mut lines[sec_start + 1..sec_end] {
            if line.trim_start().starts_with("FrameRateLimit=") {
                *line = format!("FrameRateLimit={}", fps_val);
                found_fps = true;
            } else if line.trim_start().starts_with("bUseVSync=") {
                *line = format!("bUseVSync={}", vsync_val);
                found_vsync = true;
            }
        }

        let mut insert_pos = sec_end;
        if !found_fps {
            lines.insert(insert_pos, format!("FrameRateLimit={}", fps_val));
            insert_pos += 1;
        }
        if !found_vsync {
            lines.insert(insert_pos, format!("bUseVSync={}", vsync_val));
        }

        let mut out = lines.join("\n");
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out
    }

    /// Clear DXVK and VKD3D shader cache to resolve stuttering after updates
    pub fn clear_shader_cache(prefix_dir: &Path, game_dir: &Path) -> Result<usize> {
        let mut count = 0;
        let cache_dirs = [prefix_dir.join("shadercache"), game_dir.to_path_buf()];

        for dir in &cache_dirs {
            if dir.exists() {
                if let Ok(entries) = fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                            if matches!(ext, "dxvk-cache" | "vkd3d-cache" | "cache")
                                && fs::remove_file(&path).is_ok()
                            {
                                count += 1;
                            }
                        }
                    }
                }
            }
        }

        Ok(count)
    }
}

pub fn which_exists(bin: &str) -> bool {
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

#[cfg(test)]
mod tests {
    use super::SystemTweaks;

    #[test]
    fn patch_gus_inserta_claves_dentro_de_seccion_correcta() {
        let ini = "[/Script/Engine.GameUserSettings]\nResolutionSizeX=1920\n\n[ScalabilityGroups]\nsg.ViewDistanceQuality=3\n";
        let patched = SystemTweaks::patch_game_user_settings_ini(ini, 144, false);
        let pos_fps = patched.find("FrameRateLimit=144.000000").unwrap();
        let pos_vsync = patched.find("bUseVSync=False").unwrap();
        let pos_scalability = patched.find("[ScalabilityGroups]").unwrap();
        assert!(pos_fps < pos_scalability);
        assert!(pos_vsync < pos_scalability);
    }
}
