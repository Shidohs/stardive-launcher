mod api;
mod auth;
mod config;
mod downloader;
mod installer;
mod runner;
mod tweaks;

use anyhow::Result;
use clap::Parser;
use config::LauncherConfig;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::Emitter;
use tokio::sync::mpsc;

#[derive(Parser, Debug)]
#[command(name = "stardive-launcher")]
#[command(author = "Shidox")]
#[command(version = "1.0.0")]
#[command(about = "Native Linux Launcher & Downloader for Mongil: Star Dive", long_about = None)]
struct Cli {
    /// Inicia el juego directamente con Proton
    #[arg(short, long)]
    play: bool,

    /// Descarga e instala la última versión del cliente headless
    #[arg(short, long)]
    download: bool,

    /// Verifica el estado de la instalación local y la versión remota
    #[arg(short, long)]
    status: bool,

    /// Busca y aplica actualizaciones si están disponibles
    #[arg(short, long)]
    update: bool,

    /// Actualiza la configuración de Lutris existente
    #[arg(long)]
    update_lutris: bool,
}

#[derive(Clone)]
pub struct AppState {
    cancel_token: Arc<AtomicBool>,
    game_process: Arc<Mutex<Option<Child>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            cancel_token: Arc::new(AtomicBool::new(false)),
            game_process: Arc::new(Mutex::new(None)),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct LauncherStatusPayload {
    pub is_installed: bool,
    pub installed_version: String,
    pub server_version: String,
    pub runner_name: String,
    pub has_gamemode: bool,
    pub is_downloading: bool,
    pub is_playing: bool,
    pub is_authenticated: bool,
    pub player_name: String,
    pub profile_img_url: String,
    pub player_id: String,
    pub channel: String,
}

#[derive(Serialize, Deserialize)]
pub struct ConfigAndRunnersPayload {
    pub config: LauncherConfig,
    pub runners: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct DownloadProgressPayload {
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub speed_bytes_per_sec: f64,
    pub progress_ratio: f32,
    pub eta_seconds: Option<u64>,
    pub status: String,
    pub is_completed: bool,
}

// ------------------------------------------------------------
// TAURI IPC COMMANDS
// ------------------------------------------------------------

#[tauri::command]
async fn get_launcher_status(
    state: tauri::State<'_, AppState>,
) -> Result<LauncherStatusPayload, String> {
    let config = LauncherConfig::load().map_err(|e| e.to_string())?;
    let is_installed = config.is_installed();

    let manifest = installer::Installer::read_manifest(&config.game_dir);
    let installed_version = manifest.map(|m| m.version).unwrap_or_else(|| {
        if is_installed {
            "1.03.00".to_string()
        } else {
            "1.03.00".to_string()
        }
    });

    let runner_name = config
        .proton_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let has_gamemode = tweaks::SystemTweaks::has_gamemode();
    let is_playing = runner::GameRunner::is_running(&mut *state.game_process.lock().unwrap());
    let auth_status = auth::AuthManager::get_status();

    Ok(LauncherStatusPayload {
        is_installed,
        installed_version,
        server_version: "1.03.00".to_string(),
        runner_name,
        has_gamemode,
        is_downloading: false,
        is_playing,
        is_authenticated: auth_status.is_logged_in,
        player_name: auth_status.profile_name,
        profile_img_url: auth_status.profile_img_url,
        player_id: auth_status.player_id,
        channel: auth_status.channel,
    })
}

#[tauri::command]
async fn start_download(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let config = LauncherConfig::load().map_err(|e| e.to_string())?;
    state.cancel_token.store(false, Ordering::Relaxed);
    let cancel_token = state.cancel_token.clone();

    let (tx, mut rx) = mpsc::channel(100);

    // Spawn download worker task
    let game_dir = config.game_dir.clone();
    let prefix_dir = config.prefix_dir.clone();

    tokio::spawn(async move {
        let api = api::ApiClient::new();
        let build_info = match api.fetch_build_info().await {
            Ok(b) => b,
            Err(e) => {
                let _ = tx
                    .send(downloader::DownloadProgress {
                        downloaded_bytes: 0,
                        total_bytes: 0,
                        speed_bytes_per_sec: 0.0,
                        progress_ratio: 0.0,
                        eta_seconds: None,
                        status: format!("Error de conexión: {}", e),
                        is_completed: false,
                    })
                    .await;
                return;
            }
        };

        let download_dest = prefix_dir.join("stardive_client.zip");

        match downloader::Downloader::download(
            &build_info.download_url,
            &download_dest,
            &build_info.file_hash,
            tx.clone(),
            cancel_token.clone(),
        )
        .await
        {
            Ok(zip_path) => {
                let _ = tx
                    .send(downloader::DownloadProgress {
                        downloaded_bytes: build_info.file_size,
                        total_bytes: build_info.file_size,
                        speed_bytes_per_sec: 0.0,
                        progress_ratio: 1.0,
                        eta_seconds: Some(0),
                        status: "Extrayendo archivos del cliente (1.3 GB)...".to_string(),
                        is_completed: true,
                    })
                    .await;

                let extract_result = installer::Installer::extract_zip(
                    &zip_path,
                    &game_dir,
                    &build_info.version,
                    build_info.build_sequence,
                    None,
                    Some(cancel_token),
                );

                if let Err(err) = extract_result {
                    eprintln!("Error al extraer zip: {}", err);
                } else {
                    let _ = std::fs::remove_file(&zip_path);
                }
            }
            Err(err) => {
                eprintln!("Error en la descarga: {}", err);
            }
        }
    });

    // Stream progress events to webview window
    let app_handle_clone = app_handle.clone();
    tokio::spawn(async move {
        while let Some(prog) = rx.recv().await {
            let payload = DownloadProgressPayload {
                downloaded_bytes: prog.downloaded_bytes,
                total_bytes: prog.total_bytes,
                speed_bytes_per_sec: prog.speed_bytes_per_sec,
                progress_ratio: prog.progress_ratio,
                eta_seconds: prog.eta_seconds,
                status: prog.status,
                is_completed: prog.is_completed,
            };
            let _ = app_handle_clone.emit("download-progress", &payload);
            if payload.is_completed {
                break;
            }
        }
    });

    Ok(())
}

#[tauri::command]
fn cancel_download(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.cancel_token.store(true, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
fn launch_game(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let config = LauncherConfig::load().map_err(|e| e.to_string())?;

    let child = runner::GameRunner::launch(&config).map_err(|e| e.to_string())?;
    *state.game_process.lock().unwrap() = Some(child);

    let process_lock = state.game_process.clone();
    let app_handle_clone = app_handle.clone();

    // Spawn monitoring thread
    std::thread::spawn(move || {
        let _ = app_handle_clone.emit("game-status", serde_json::json!({ "is_running": true }));

        loop {
            std::thread::sleep(std::time::Duration::from_millis(800));
            let mut proc_opt = process_lock.lock().unwrap();
            if !runner::GameRunner::is_running(&mut *proc_opt) {
                let _ = app_handle_clone
                    .emit("game-status", serde_json::json!({ "is_running": false }));
                break;
            }
        }
    });

    Ok(())
}

#[tauri::command]
fn get_config_and_runners() -> Result<ConfigAndRunnersPayload, String> {
    let config = LauncherConfig::load().map_err(|e| e.to_string())?;
    let runners = tweaks::SystemTweaks::detect_proton_runners()
        .into_iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    Ok(ConfigAndRunnersPayload { config, runners })
}

#[tauri::command]
fn save_config(config: LauncherConfig) -> Result<(), String> {
    config.save().map_err(|e| e.to_string())
}

#[tauri::command]
fn apply_fps_tweak(fps_limit: u32, vsync: bool) -> Result<(), String> {
    let config = LauncherConfig::load().map_err(|e| e.to_string())?;
    tweaks::SystemTweaks::apply_fps_tweak(&config.prefix_dir, fps_limit, vsync)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn clear_shader_cache() -> Result<usize, String> {
    let config = LauncherConfig::load().map_err(|e| e.to_string())?;
    tweaks::SystemTweaks::clear_shader_cache(&config.prefix_dir).map_err(|e| e.to_string())
}

#[tauri::command]
fn rewrite_manifest() -> Result<(), String> {
    let config = LauncherConfig::load().map_err(|e| e.to_string())?;
    installer::Installer::write_manifest(&config.game_dir, "1.03.00", 61).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_folder(target: String) -> Result<(), String> {
    let config = LauncherConfig::load().map_err(|e| e.to_string())?;
    let path = if target == "prefix" {
        config.prefix_dir
    } else {
        config.game_dir
    };
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    Ok(())
}

#[tauri::command]
fn open_external(url: String) -> Result<(), String> {
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    Ok(())
}

#[tauri::command]
fn get_logs() -> Result<String, String> {
    let config = LauncherConfig::load().map_err(|e| e.to_string())?;
    std::fs::read_to_string(config.log_file_path()).map_err(|e| e.to_string())
}

#[tauri::command]
fn clear_logs() -> Result<(), String> {
    let config = LauncherConfig::load().map_err(|e| e.to_string())?;
    std::fs::write(config.log_file_path(), "").map_err(|e| e.to_string())
}

#[tauri::command]
fn get_auth_state() -> Result<auth::AuthStatusPayload, String> {
    Ok(auth::AuthManager::get_status())
}

#[tauri::command]
async fn start_auth_flow(
    app_handle: tauri::AppHandle,
    channel: String,
) -> Result<String, String> {
    let (port, rx) = auth::AuthManager::listen_for_auth_callback(55000)
        .await
        .map_err(|e| e.to_string())?;

    let auth_url = auth::AuthManager::build_auth_url(&channel, port, "es");
    println!("Abriendo Netmarble Members SSO en navegador: {}", auth_url);

    let _ = std::process::Command::new("xdg-open").arg(&auth_url).spawn();

    let app_handle_clone = app_handle.clone();

    tokio::spawn(async move {
        match rx.await {
            Ok(session) => {
                println!("✓ Sesión oficial Netmarble NM recibida para {}", session.profile_name);
                let status = auth::AuthManager::get_status();
                let _ = app_handle_clone.emit(
                    "auth-status-changed",
                    serde_json::json!({
                        "success": true,
                        "status": status,
                        "profile_name": session.profile_name
                    }),
                );
            }
            Err(_) => {
                eprintln!("Listener de autenticación cerrado o expirado.");
                let _ = app_handle_clone.emit(
                    "auth-status-changed",
                    serde_json::json!({
                        "success": false,
                        "error": "Cancelado o timeout superado."
                    }),
                );
            }
        }
    });

    Ok(auth_url)
}

#[tauri::command]
async fn manual_auth(
    app_handle: tauri::AppHandle,
    input: String,
) -> Result<auth::AuthStatusPayload, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("El token, URL o payload no puede estar vacío.".to_string());
    }

    let session = auth::AuthManager::process_auth_result(trimmed)
        .await
        .map_err(|e| e.to_string())?;

    let status = auth::AuthManager::get_status();
    let _ = app_handle.emit(
        "auth-status-changed",
        serde_json::json!({
            "success": true,
            "status": status,
            "profile_name": session.profile_name
        }),
    );

    Ok(status)
}

#[tauri::command]
fn logout(app_handle: tauri::AppHandle) -> Result<(), String> {
    auth::AuthManager::clear_session().map_err(|e| e.to_string())?;
    let _ = app_handle.emit(
        "auth-status-changed",
        serde_json::json!({
            "success": true,
            "status": auth::AuthStatusPayload::default()
        }),
    );
    Ok(())
}

// ------------------------------------------------------------
// MAIN ENTRY POINT (CLI & GUI)
// ------------------------------------------------------------

fn main() -> Result<()> {
    let args = Cli::parse();
    let config = LauncherConfig::load()?;

    if args.status {
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(print_status(&config))?;
        return Ok(());
    }

    if args.update_lutris {
        update_lutris_config(&config)?;
        return Ok(());
    }

    if args.download || args.update {
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(run_headless_download_and_install(&config))?;
        return Ok(());
    }

    if args.play {
        run_headless_play(&config)?;
        return Ok(());
    }

    // Default: Launch Tauri GUI
    tauri::Builder::default()
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            get_launcher_status,
            start_download,
            cancel_download,
            launch_game,
            get_config_and_runners,
            save_config,
            apply_fps_tweak,
            clear_shader_cache,
            rewrite_manifest,
            open_folder,
            open_external,
            get_logs,
            clear_logs,
            get_auth_state,
            start_auth_flow,
            manual_auth,
            logout,
        ])
        .run(tauri::generate_context!())
        .expect("Error al iniciar Tauri GUI");

    Ok(())
}

async fn print_status(config: &LauncherConfig) -> Result<()> {
    println!("============================================================");
    println!("⭐ STARDIVE LAUNCHER - ESTADO DEL SISTEMA");
    println!("============================================================");
    println!("Configuración:");
    println!(
        "  Ruta de Config:    {:?}",
        LauncherConfig::config_file_path()
    );
    println!("  Prefijo Wine:      {:?}", config.prefix_dir);
    println!("  Carpeta Juego:     {:?}", config.game_dir);
    println!("  Ejecutable:        {:?}", config.game_exe_path());
    println!("  Proton Runner:     {:?}", config.proton_bin());
    println!("  Argumentos:        {}", config.launch_args);
    println!("------------------------------------------------------------");

    println!("Instalación Local:");
    if config.is_installed() {
        println!("  Estado:            INSTALADO ✓");
        if let Some(manifest) = installer::Installer::read_manifest(&config.game_dir) {
            println!("  Versión Local:     {}", manifest.version);
            println!("  Secuencia Build:   {}", manifest.build_sequence);
            println!("  Fecha Instalación: {}", manifest.installed_at);
        } else {
            println!("  Versión Local:     Archivos presentes (sin manifest)");
        }
    } else {
        println!("  Estado:            NO INSTALADO ✗");
    }

    println!("------------------------------------------------------------");
    println!("Consultando Netmarble API...");
    let api = api::ApiClient::new();
    match api.fetch_build_info().await {
        Ok(build) => {
            println!("  Versión Servidor:  {}", build.version);
            println!("  Build Sequence:    {}", build.build_sequence);
            println!(
                "  Tamaño Descarga:   {:.1} MB",
                build.file_size as f64 / (1024.0 * 1024.0)
            );
            println!("  MD5 Hash:          {}", build.file_hash);
            println!("  URL CDN:           {}", build.download_url);

            let local_seq =
                installer::Installer::read_manifest(&config.game_dir).map(|m| m.build_sequence);

            if let Some(seq) = local_seq {
                if build.build_sequence > seq {
                    println!(
                        "\n  >>> ¡NUEVA ACTUALIZACIÓN DISPONIBLE! ({} -> {}) <<<",
                        seq, build.build_sequence
                    );
                    println!("  Ejecuta con --update para actualizar.");
                } else {
                    println!("\n  El juego está actualizado a la última versión.");
                }
            } else if !config.is_installed() {
                println!("\n  Ejecuta con --download para descargar e instalar.");
            }
        }
        Err(err) => {
            println!("  Error al consultar API: {}", err);
        }
    }
    println!("============================================================");
    Ok(())
}

async fn run_headless_download_and_install(config: &LauncherConfig) -> Result<()> {
    println!("Consultando datos de build a Netmarble...");
    let api = api::ApiClient::new();
    let build = api.fetch_build_info().await?;

    println!(
        "Versión: {} (Build {})",
        build.version, build.build_sequence
    );
    println!("Descargando desde CDN: {}", build.download_url);

    let zip_dest = config.prefix_dir.join("stardive_client.zip");
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, mut rx) = mpsc::channel(50);

    let download_handle = tokio::spawn(async move {
        downloader::Downloader::download(
            &build.download_url,
            &zip_dest,
            &build.file_hash,
            tx,
            cancel,
        )
        .await
    });

    while let Some(prog) = rx.recv().await {
        if prog.is_completed {
            println!("\n✓ {}", prog.status);
            break;
        }
        print!(
            "\r[{:3.1}%] {} / {} | {} | ETA: {} | {}",
            prog.progress_ratio * 100.0,
            format!("{:.1} MB", prog.downloaded_mb()),
            format!("{:.1} MB", prog.total_mb()),
            prog.speed_formatted(),
            prog.eta_formatted(),
            prog.status
        );
        use std::io::Write;
        let _ = std::io::stdout().flush();
    }

    let downloaded_file = download_handle.await??;

    println!("\nExtrayendo archivos a {:?}...", config.game_dir);
    installer::Installer::extract_zip(
        &downloaded_file,
        &config.game_dir,
        &build.version,
        build.build_sequence,
        None,
        None,
    )?;

    let _ = std::fs::remove_file(&downloaded_file);
    println!("✓ ¡Instalación completada exitosamente!");
    Ok(())
}

fn run_headless_play(config: &LauncherConfig) -> Result<()> {
    let mut child = runner::GameRunner::launch(config)?;
    println!(
        "Mongil: Star Dive iniciado (PID: {}). Esperando finalización...",
        child.id()
    );
    let status = child.wait()?;
    println!("El proceso del juego finalizó con: {:?}", status);
    Ok(())
}

fn update_lutris_config(config: &LauncherConfig) -> Result<()> {
    let lutris_games_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/home/shidox"))
        .join(".config")
        .join("lutris")
        .join("games");

    println!(
        "Buscando configuraciones de Lutris en {:?}",
        lutris_games_dir
    );
    if !lutris_games_dir.exists() {
        println!("No se encontró la carpeta de configuración de Lutris.");
        return Ok(());
    }

    let entries = std::fs::read_dir(&lutris_games_dir)?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("yml") {
            let content = std::fs::read_to_string(&path)?;
            if content.contains("mongil")
                || content.contains("stardive")
                || content.contains("Netmarble")
            {
                println!("Encontrada configuración de Lutris en: {:?}", path);
                let exe_str = config.game_exe_path().to_string_lossy().to_string();
                let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
                let mut updated = false;

                for line in &mut lines {
                    if line.trim_start().starts_with("exe:") {
                        *line = format!("  exe: {}", exe_str);
                        updated = true;
                    }
                    if line.trim_start().starts_with("args:") {
                        *line = format!("  args: {}", config.launch_args);
                    }
                }

                if updated {
                    std::fs::write(&path, lines.join("\n"))?;
                    println!(
                        "✓ Configuración de Lutris actualizada para apuntar a: {}",
                        exe_str
                    );
                }
            }
        }
    }
    Ok(())
}
