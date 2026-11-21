use crate::auth::AuthSession;
use crate::config::{LauncherConfig, RenderApi};
use anyhow::{anyhow, Context, Result};
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

pub struct GameRunner;

impl GameRunner {
    pub fn launch(
        config: &LauncherConfig,
        session_override: Option<&AuthSession>,
    ) -> Result<Child> {
        let game_exe = config.game_exe_path();
        if !game_exe.exists() {
            return Err(anyhow!(
                "El ejecutable del juego no existe en: {:?}. Por favor descarga el juego primero.",
                game_exe
            ));
        }

        let proton_bin = config.proton_bin();
        if !config.use_umu && !proton_bin.exists() {
            return Err(anyhow!(
                "El ejecutable de Proton no fue encontrado en: {:?}. Verifica la ruta de GE-Proton en la configuración.",
                proton_bin
            ));
        }

        let prefix_dir = &config.prefix_dir;
        let game_dir = &config.game_dir;
        let log_path = config.log_file_path();

        let log_file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&log_path)
            .with_context(|| format!("No se pudo abrir el archivo de log en {:?}", log_path))?;

        let log_err_file = log_file
            .try_clone()
            .context("No se pudo clonar el handle del archivo de log")?;

        // Aplicar automáticamente optimizaciones de Unreal Engine 5 (FPS unlocker, motion blur, etc.)
        let ue5_tweaks = crate::tweaks::Ue5Tweaks {
            max_fps: config.fps_limit,
            vsync: config.vsync,
            disable_motion_blur: config.disable_motion_blur,
            disable_chromatic_aberration: config.disable_chromatic_aberration,
            disable_film_grain: config.disable_film_grain,
        };
        let _ = crate::tweaks::SystemTweaks::apply_user_tweaks(prefix_dir, &ue5_tweaks);

        // Determinar comando base con wrappers (gamescope / gamemoderun / umu-run / proton)
        let mut cmd = if config.use_gamescope && crate::tweaks::SystemTweaks::has_gamescope() {
            let mut c = Command::new("gamescope");
            for arg in config.gamescope_args.split_whitespace() {
                c.arg(arg);
            }
            c.arg("--");
            if config.use_gamemode && crate::tweaks::SystemTweaks::has_gamemode() {
                c.arg("gamemoderun");
            }
            if config.use_umu {
                c.arg("umu-run");
            } else {
                c.arg(&proton_bin);
                c.arg("run");
            }
            c.arg(&game_exe);
            c
        } else if config.use_gamemode && crate::tweaks::SystemTweaks::has_gamemode() {
            let mut c = Command::new("gamemoderun");
            if config.use_umu {
                c.arg("umu-run");
            } else {
                c.arg(&proton_bin);
                c.arg("run");
            }
            c.arg(&game_exe);
            c
        } else if config.use_umu {
            let mut c = Command::new("umu-run");
            c.arg(&game_exe);
            c
        } else {
            let mut c = Command::new(&proton_bin);
            c.arg("run");
            c.arg(&game_exe);
            c
        };

        // Configuración de API gráfica para Unreal Engine 5 (DirectX 12 vs DirectX 11)
        let render_api = config.render_api.unwrap_or_default();
        match render_api {
            RenderApi::DirectX12 => {
                cmd.arg("-dx12");
                cmd.env("VKD3D_CONFIG", "dxr11");
                cmd.env("PROTON_USE_WINED3D", "0");
            }
            RenderApi::DirectX11 => {
                cmd.arg("-dx11");
                cmd.env("DXVK_ASYNC", "1");
                cmd.env("DXVK_GPLASYNCCACHE", "1");
            }
        }

        // Netmarble Official SSO Authentication Token Injection
        let mut has_nmauth = false;
        let active_session = session_override
            .cloned()
            .or_else(|| crate::auth::AuthManager::load_session().ok().flatten());
        if let Some(session) = active_session {
            println!(
                "Inyectando token de sesión oficial de Netmarble: {} (Player ID: {})",
                session.profile_name, session.player_id
            );
            // Sincronizar clave de dispositivo oficial en el registro de Wine (HKCU\Software\Netmarble\NetmarbleSDK)
            ensure_wine_registry_device_key(prefix_dir, &session.device_key);

            // Inyectar argumentos en formato oficial para Unreal Engine / NetmarbleSDK (platformAuthType: NM)
            cmd.arg("NMAUTH_TYPE=netmarble");
            cmd.arg("-NMAUTH_TYPE=netmarble");
            cmd.arg(format!("NMAUTH_TOKEN={}", session.launcher_token));
            cmd.arg(format!("-NMAUTH_TOKEN={}", session.launcher_token));

            // Variables de entorno de Wine por si el SDK las consulta vía GetEnvironmentVariableW
            cmd.env("NMAUTH_TYPE", "netmarble");
            cmd.env("NMAUTH_TOKEN", &session.launcher_token);
            cmd.env("NMDeviceKey", &session.device_key);
            cmd.env("DEVICE_KEY", &session.device_key);
            cmd.env("NMENV", "nmp");
            has_nmauth = true;
        } else {
            eprintln!("ADVERTENCIA: No se detectó sesión iniciada en Netmarble. STARDIVE.exe solicitará inicio de sesión.");
        }

        let mut has_nmenv = false;
        // Add launch arguments (e.g. "NMENV=nmp") or detect prefixed env vars
        for arg in config.launch_args.split_whitespace() {
            if arg.starts_with("NMENV=") || arg.starts_with("-NMENV=") {
                has_nmenv = true;
            }
            if (arg.starts_with("NMAUTH_TYPE=")
                || arg.starts_with("NMAUTH_TOKEN=")
                || arg.starts_with("-NMAUTH_TYPE=")
                || arg.starts_with("-NMAUTH_TOKEN="))
                && has_nmauth
            {
                // Ya inyectado dinámicamente desde auth.json
                continue;
            }
            if let Some((k, v)) = arg.split_once('=') {
                let k_upper = k.to_uppercase();
                if k_upper.starts_with("PROTON_")
                    || k_upper.starts_with("DXVK_")
                    || k_upper.starts_with("WINE")
                {
                    let mut val = v.to_string();
                    if val.eq_ignore_ascii_case("false") {
                        val = "0".to_string();
                    } else if val.eq_ignore_ascii_case("true") {
                        val = "1".to_string();
                    }
                    cmd.env(k, val);
                    continue;
                }
            }
            cmd.arg(arg);
        }

        if !has_nmenv {
            cmd.arg("NMENV=nmp");
            cmd.arg("-NMENV=nmp");
        }

        // Directorio de trabajo
        cmd.current_dir(game_dir);

        // Heredar variables de pantalla y sesión
        if let Ok(disp) = std::env::var("DISPLAY") {
            cmd.env("DISPLAY", disp);
        } else {
            cmd.env("DISPLAY", ":0");
        }
        if let Ok(xauth) = std::env::var("XAUTHORITY") {
            cmd.env("XAUTHORITY", xauth);
        }
        if let Ok(wayland) = std::env::var("WAYLAND_DISPLAY") {
            cmd.env("WAYLAND_DISPLAY", wayland);
        }

        // Configuración de Proton y UMU
        cmd.env("STEAM_COMPAT_DATA_PATH", prefix_dir);
        let steam_client_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".local")
            .join("share")
            .join("Steam");
        cmd.env("STEAM_COMPAT_CLIENT_INSTALL_PATH", &steam_client_dir);
        cmd.env("WINEPREFIX", prefix_dir);
        cmd.env("PROTONPATH", &config.proton_path);
        cmd.env("GAMEID", "umu-stardive");

        if config.use_mangohud {
            cmd.env("MANGOHUD", "1");
            cmd.env("MANGOHUD_DLSYM", "1");
        }

        // Flags de rendimiento y compatibilidad
        if config.dxvk_async {
            cmd.env("DXVK_ASYNC", "1");
        }
        if config.wine_esync {
            cmd.env("WINEESYNC", "1");
        }
        if config.wine_fsync {
            cmd.env("WINEFSYNC", "1");
        }
        if config.nvapi || config.enable_nvapi_dlss {
            cmd.env("PROTON_ENABLE_NVAPI", "1");
            cmd.env("DXVK_ENABLE_NVAPI", "1");
        }
        if config.enable_wayland {
            cmd.env("PROTON_ENABLE_WAYLAND", "1");
        }

        // Proton WineD3D setting (false/0 para forzar DXVK/VKD3D necesario para UE5)
        if config.proton_use_wined3d {
            cmd.env("PROTON_USE_WINED3D", "1");
        } else {
            cmd.env("PROTON_USE_WINED3D", "0");
        }

        // Variables de entorno personalizadas del usuario
        for (k, v) in &config.custom_env {
            let mut val = v.clone();
            if k.eq_ignore_ascii_case("PROTON_USE_WINED3D") {
                if val.eq_ignore_ascii_case("false") || val == "0" {
                    val = "0".to_string();
                } else if val.eq_ignore_ascii_case("true") || val == "1" {
                    val = "1".to_string();
                }
            }
            cmd.env(k, val);
        }

        cmd.stdout(Stdio::from(log_file));
        cmd.stderr(Stdio::from(log_err_file));

        println!("Iniciando Mongil: Star Dive...");
        println!("Proton: {:?}", proton_bin);
        println!("Game EXE: {:?}", game_exe);
        println!("Args: {}", config.launch_args);
        println!("Wine Prefix: {:?}", prefix_dir);
        println!("Logs: {:?}", log_path);

        let child = cmd.spawn().with_context(|| {
            format!(
                "Fallo al ejecutar el juego con: {:?}",
                if config.use_umu { "umu-run" } else { "proton" }
            )
        })?;

        Ok(child)
    }
}

/// Sincroniza la clave NMDeviceKey en el registro de Wine (HKCU\Software\Netmarble\NetmarbleSDK)
/// Requerida por Unreal Engine y NetmarbleSDK para validar el token JWT oficial
fn ensure_wine_registry_device_key(prefix_dir: &std::path::Path, device_key: &str) {
    let candidates = [
        prefix_dir.join("pfx").join("user.reg"),
        prefix_dir.join("user.reg"),
    ];
    let Some(user_reg) = candidates.into_iter().find(|p| p.exists()) else {
        return;
    };

    if let Ok(content) = std::fs::read_to_string(&user_reg) {
        let updated = upsert_wine_registry_device_key(&content, device_key);
        if updated != content {
            let _ = std::fs::write(&user_reg, updated);
        }
    }
}

fn upsert_wine_registry_device_key(content: &str, device_key: &str) -> String {
    let section_header = "[Software\\\\Netmarble\\\\NetmarbleSDK]";
    let entry = format!("\"NMDeviceKey\"=\"{}\"", device_key);

    if let Some(sec_idx) = content.find(section_header) {
        let rest = &content[sec_idx..];
        let sec_end = rest[section_header.len()..]
            .find("\n[")
            .map(|o| sec_idx + section_header.len() + o)
            .unwrap_or(content.len());
        let sec_content = &content[sec_idx..sec_end];

        if sec_content.lines().any(|l| l.trim() == entry) {
            return content.to_string();
        }

        let mut out = content.to_string();
        if sec_content
            .lines()
            .any(|l| l.starts_with("\"NMDeviceKey\"="))
        {
            let lines: Vec<String> = sec_content
                .lines()
                .map(|l| {
                    if l.starts_with("\"NMDeviceKey\"=") {
                        entry.clone()
                    } else {
                        l.to_string()
                    }
                })
                .collect();
            let mut replaced = lines.join("\n");
            if sec_content.ends_with('\n') {
                replaced.push('\n');
            }
            out.replace_range(sec_idx..sec_end, &replaced);
        } else if let Some(first_nl) = sec_content.find('\n') {
            let mut insert_offset = sec_idx + first_nl + 1;
            let after_header = &content[insert_offset..sec_end];
            if after_header.starts_with("#time=") {
                if let Some(time_nl) = after_header.find('\n') {
                    insert_offset += time_nl + 1;
                } else {
                    insert_offset = sec_end;
                }
            }
            out.insert_str(insert_offset, &format!("{}\n", entry));
        } else {
            out.push_str(&format!("\n{}\n", entry));
        }
        return out;
    }

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(1789048653);
    let mut out = content.to_string();
    out.push_str(&format!(
        "\n{} {}\n#time=1db000000000000\n{}\n",
        section_header, ts, entry
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::{ensure_wine_registry_device_key, upsert_wine_registry_device_key};
    use std::fs;

    #[test]
    fn upsert_preserva_timestamp_en_cabecera_sin_clave() {
        let reg = "[Software\\\\Netmarble\\\\NetmarbleSDK] 1789100489\n#time=1dd41a504a2e832\n\n[Software\\\\Valve\\\\Steam] 1787951530\n";
        let updated = upsert_wine_registry_device_key(reg, "uuid-test-123");
        assert!(updated.contains("[Software\\\\Netmarble\\\\NetmarbleSDK] 1789100489\n#time=1dd41a504a2e832\n\"NMDeviceKey\"=\"uuid-test-123\"\n"));
        assert!(updated.contains("[Software\\\\Valve\\\\Steam] 1787951530"));
    }

    #[test]
    fn upsert_reemplaza_en_ultima_seccion_sin_duplicar() {
        let reg =
            "[Software\\\\Netmarble\\\\NetmarbleSDK] 1789100489\n\"NMDeviceKey\"=\"old-key\"\n";
        let updated = upsert_wine_registry_device_key(reg, "new-key");
        assert_eq!(updated.matches("\"NMDeviceKey\"=").count(), 1);
        assert!(updated.contains("\"NMDeviceKey\"=\"new-key\""));
    }

    #[test]
    fn ensure_resuelve_pfx_user_reg() {
        let base = std::env::temp_dir().join(format!("stardive_test_pfx_{}", std::process::id()));
        let pfx = base.join("pfx");
        fs::create_dir_all(&pfx).unwrap();
        let reg_path = pfx.join("user.reg");
        fs::write(
            &reg_path,
            "[Software\\\\Netmarble\\\\NetmarbleSDK] 1789100489\n",
        )
        .unwrap();

        ensure_wine_registry_device_key(&base, "pfx-uuid-999");
        let content = fs::read_to_string(&reg_path).unwrap();
        assert!(content.contains("[Software\\\\Netmarble\\\\NetmarbleSDK] 1789100489\n\"NMDeviceKey\"=\"pfx-uuid-999\"\n"));

        let _ = fs::remove_dir_all(&base);
    }
}
