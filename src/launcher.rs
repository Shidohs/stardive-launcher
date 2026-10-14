use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use serde::{Deserialize, Serialize};
use tauri::Window;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum RenderApi {
    DirectX12, // Usa VKD3D-Proton (Recomendado para UE5)
    DirectX11, // Usa DXVK / DXVK-Async
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LaunchConfig {
    pub game_exe: PathBuf,
    pub prefix_path: PathBuf,
    pub proton_path: PathBuf,
    pub render_api: RenderApi,
    pub use_umu: bool,
    pub enable_mangohud: bool,
    pub enable_gamemode: bool,
    pub enable_nvapi_dlss: bool,
    pub enable_wayland: bool,
    #[serde(alias = "close_Action")]
    pub close_action: String, // "minimize", "tray", "close"
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Ue5Tweaks {
    pub max_fps: u32,
    pub disable_motion_blur: bool,
    pub disable_chromatic_aberration: bool,
    pub disable_film_grain: bool,
}

/// Parchea el archivo Engine.ini de Unreal Engine 5 con optimizaciones y FPS unlocker
#[tauri::command]
pub fn apply_ue5_tweaks(engine_ini_path: PathBuf, tweaks: Ue5Tweaks) -> Result<(), String> {
    if let Some(parent) = engine_ini_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let mut content = if engine_ini_path.exists() {
        fs::read_to_string(&engine_ini_path).map_err(|e| e.to_string())?
    } else {
        String::new()
    };

    // Limpiar bloque previo generado por el launcher si existe
    let marker_start = "; --- STARDIVE LAUNCHER TWEAKS START ---";
    let marker_end = "; --- STARDIVE LAUNCHER TWEAKS END ---";
    if let (Some(start), Some(end)) = (content.find(marker_start), content.find(marker_end)) {
        content.replace_range(start..(end + marker_end.len()), "");
    }

    let mut block = format!("\n{}\n[/Script/Engine.Engine]\n", marker_start);
    if tweaks.max_fps > 0 {
        block.push_str(&format!("bSmoothFrameRate=False\nMinDesiredFrameRate=30.000000\nFixedFrameRate={}.000000\n", tweaks.max_fps));
    }

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
    block.push_str(&format!("{}\n", marker_end));

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&engine_ini_path)
        .map_err(|e| e.to_string())?;

    file.write_all(content.trim().as_bytes()).map_err(|e| e.to_string())?;
    file.write_all(block.as_bytes()).map_err(|e| e.to_string())?;

    Ok(())
}

/// Lanza el juego con soporte para GE-Proton, umu-launcher y perfiles DX12/DX11
#[tauri::command]
pub async fn launch_game(window: Window, config: LaunchConfig) -> Result<(), String> {
    // 1. Determinar el comando base (gamemoderun como wrapper si está activo)
    let mut cmd = if config.enable_gamemode {
        let mut c = Command::new("gamemoderun");
        if config.use_umu {
            c.arg("umu-run");
        } else {
            c.arg(config.proton_path.join("proton")).arg("waitforexitandrun");
        }
        c
    } else if config.use_umu {
        Command::new("umu-run")
    } else {
        let mut c = Command::new(config.proton_path.join("proton"));
        c.arg("waitforexitandrun");
        c
    };

    cmd.arg(&config.game_exe);

    // 2. Argumentos del motor Unreal Engine 5 según API gráfica
    match config.render_api {
        RenderApi::DirectX12 => {
            cmd.arg("-dx12");
            // Optimizaciones de VKD3D-Proton para UE5
            cmd.env("VKD3D_CONFIG", "dxr11");
            cmd.env("PROTON_USE_WINED3D", "0");
        }
        RenderApi::DirectX11 => {
            cmd.arg("-dx11");
            // Activar DXVK Async para quienes usen el modo legacy DX11
            cmd.env("DXVK_ASYNC", "1");
            cmd.env("DXVK_GPLASYNCCACHE", "1");
        }
    }

    // 3. Variables de entorno de Proton / Wine
    cmd.env("STEAM_COMPAT_DATA_PATH", &config.prefix_path);
    cmd.env("STEAM_COMPAT_CLIENT_INSTALL_PATH", "/tmp");
    cmd.env("WINEPREFIX", config.prefix_path.join("pfx"));
    cmd.env("PROTONPATH", &config.proton_path);
    cmd.env("GAMEID", "umu-stardive");

    if config.enable_mangohud {
        cmd.env("MANGOHUD", "1");
        cmd.env("MANGOHUD_DLSYM", "1");
    }
    if config.enable_nvapi_dlss {
        cmd.env("PROTON_ENABLE_NVAPI", "1");
        cmd.env("DXVK_ENABLE_NVAPI", "1");
    }
    if config.enable_wayland {
        cmd.env("PROTON_ENABLE_WAYLAND", "1");
    }

    // 4. Gestión de la ventana de Tauri para ahorrar RAM/VRAM
    match config.close_action.as_str() {
        "minimize" => { let _ = window.minimize(); }
        "tray" => { let _ = window.hide(); }
        "close" => {
            cmd.spawn().map_err(|e| format!("Error al iniciar el juego: {}", e))?;
            std::process::exit(0);
        }
        _ => {}
    }

    // Ejecutar y esperar en un hilo asíncrono para restaurar la ventana al cerrar el juego
    let mut child = cmd.spawn().map_err(|e| format!("Error al iniciar el juego: {}", e))?;
    tokio::task::spawn_blocking(move || {
        let _ = child.wait();
        let _ = window.show();
        let _ = window.set_focus();
    });

    Ok(())
}