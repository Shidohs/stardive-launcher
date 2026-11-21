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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::Emitter;
use tokio::sync::mpsc;

#[derive(Parser, Debug)]
#[command(name = "stardive-launcher")]
#[command(author = "Shidox")]
#[command(version)]
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
    is_downloading: Arc<AtomicBool>,
    is_playing: Arc<AtomicBool>,
    ephemeral_session: Arc<Mutex<Option<auth::AuthSession>>>,
    auth_cancel: Arc<Mutex<Option<Arc<tokio::sync::Notify>>>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        Self {
            cancel_token: Arc::new(AtomicBool::new(false)),
            is_downloading: Arc::new(AtomicBool::new(false)),
            is_playing: Arc::new(AtomicBool::new(false)),
            ephemeral_session: Arc::new(Mutex::new(None)),
            auth_cancel: Arc::new(Mutex::new(None)),
        }
    }

    pub fn current_auth_status(&self) -> auth::AuthStatusPayload {
        if let Some(session) = self
            .ephemeral_session
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            return auth::AuthManager::status_from_session(session);
        }
        auth::AuthManager::get_status()
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
            "Desconocida".to_string()
        } else {
            "No instalado".to_string()
        }
    });

    let runner_name = config
        .proton_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let has_gamemode = tweaks::SystemTweaks::has_gamemode();
    let is_playing = state.is_playing.load(Ordering::Relaxed);
    let is_downloading = state.is_downloading.load(Ordering::Relaxed);
    let auth_status = state.current_auth_status();

    Ok(LauncherStatusPayload {
        is_installed,
        installed_version: installed_version.clone(),
        server_version: installed_version,
        runner_name,
        has_gamemode,
        is_downloading,
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
    if state
        .is_downloading
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("Ya existe una descarga o instalación en curso.".to_string());
    }

    let config = match LauncherConfig::load() {
        Ok(c) => c,
        Err(e) => {
            state.is_downloading.store(false, Ordering::SeqCst);
            return Err(e.to_string());
        }
    };
    state.cancel_token.store(false, Ordering::Relaxed);
    let cancel_token = state.cancel_token.clone();
    let is_downloading = state.is_downloading.clone();

    let (tx, mut rx) = mpsc::channel(100);

    // Spawn download + extraction worker task
    let game_dir = config.game_dir.clone();
    let prefix_dir = config.prefix_dir.clone();
    let app_handle_err = app_handle.clone();

    tokio::spawn(async move {
        let api = api::ApiClient::new();
        let build_info = match api.fetch_build_info().await {
            Ok(b) => b,
            Err(e) => {
                let msg = format!("Error de conexión: {}", e);
                let _ = app_handle_err.emit("download-error", serde_json::json!({ "error": msg }));
                is_downloading.store(false, Ordering::SeqCst);
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
                        status: "Extrayendo archivos del cliente...".to_string(),
                        is_completed: false,
                    })
                    .await;

                let zip_for_extract = zip_path.clone();
                let game_dir_for_extract = game_dir.clone();
                let version_owned = build_info.version.clone();
                let build_seq = build_info.build_sequence;
                let cancel_for_extract = cancel_token.clone();

                let extract_result = tokio::task::spawn_blocking(move || {
                    installer::Installer::extract_zip(
                        &zip_for_extract,
                        &game_dir_for_extract,
                        &version_owned,
                        build_seq,
                        Some(cancel_for_extract),
                    )
                })
                .await;

                match extract_result {
                    Ok(Ok(())) => {
                        let _ = std::fs::remove_file(&zip_path);
                        let _ = tx
                            .send(downloader::DownloadProgress {
                                downloaded_bytes: build_info.file_size,
                                total_bytes: build_info.file_size,
                                speed_bytes_per_sec: 0.0,
                                progress_ratio: 1.0,
                                eta_seconds: Some(0),
                                status: "Instalación completada con éxito".to_string(),
                                is_completed: true,
                            })
                            .await;
                    }
                    Ok(Err(err)) => {
                        let msg = format!("Error al extraer archivos: {}", err);
                        eprintln!("{}", msg);
                        let _ = app_handle_err
                            .emit("download-error", serde_json::json!({ "error": msg }));
                    }
                    Err(join_err) => {
                        let msg = format!("Error interno durante extracción: {}", join_err);
                        eprintln!("{}", msg);
                        let _ = app_handle_err
                            .emit("download-error", serde_json::json!({ "error": msg }));
                    }
                }
            }
            Err(err) => {
                let msg = format!("Error en la descarga: {}", err);
                eprintln!("{}", msg);
                let _ = app_handle_err.emit("download-error", serde_json::json!({ "error": msg }));
            }
        }
        is_downloading.store(false, Ordering::SeqCst);
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
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if state
        .is_playing
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("El juego ya se encuentra en ejecución.".to_string());
    }

    let config = match LauncherConfig::load() {
        Ok(c) => c,
        Err(e) => {
            state.is_playing.store(false, Ordering::SeqCst);
            return Err(e.to_string());
        }
    };

    let ephemeral = state
        .ephemeral_session
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let mut child = match runner::GameRunner::launch(&config, ephemeral.as_ref()) {
        Ok(c) => c,
        Err(e) => {
            state.is_playing.store(false, Ordering::SeqCst);
            return Err(e.to_string());
        }
    };

    let close_action = config.close_action.clone();
    match close_action.as_str() {
        "minimize" | "tray" => {
            let _ = window.minimize();
        }
        "close" => {
            std::process::exit(0);
        }
        _ => {}
    }

    let is_playing = state.is_playing.clone();
    let app_handle_clone = app_handle.clone();
    let window_clone = window.clone();

    // Esperar finalización del proceso hijo sin polling activo
    std::thread::spawn(move || {
        let _ = app_handle_clone.emit("game-status", serde_json::json!({ "is_running": true }));
        let _ = child.wait();
        is_playing.store(false, Ordering::SeqCst);
        let _ = app_handle_clone.emit("game-status", serde_json::json!({ "is_running": false }));
        let _ = window_clone.unminimize();
        let _ = window_clone.show();
        let _ = window_clone.set_focus();
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
    let ue5_tweaks = tweaks::Ue5Tweaks {
        max_fps: fps_limit,
        vsync,
        disable_motion_blur: config.disable_motion_blur,
        disable_chromatic_aberration: config.disable_chromatic_aberration,
        disable_film_grain: config.disable_film_grain,
    };
    tweaks::SystemTweaks::apply_user_tweaks(&config.prefix_dir, &ue5_tweaks)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn clear_shader_cache() -> Result<usize, String> {
    let config = LauncherConfig::load().map_err(|e| e.to_string())?;
    tweaks::SystemTweaks::clear_shader_cache(&config.prefix_dir, &config.game_dir)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn rewrite_manifest() -> Result<(), String> {
    let config = LauncherConfig::load().map_err(|e| e.to_string())?;
    installer::Installer::write_manifest(&config.game_dir, "1.03.00", 61).map_err(|e| e.to_string())
}

#[tauri::command]
async fn verify_installed_game() -> Result<String, String> {
    let config = LauncherConfig::load().map_err(|e| e.to_string())?;
    let main_exe = config.game_exe_path();
    let secondary_exe = config.secondary_exe_path();

    if !main_exe.exists() {
        return Err(format!(
            "No se encontró el ejecutable principal en {:?}",
            main_exe
        ));
    }

    let mut found_count = 1;
    if secondary_exe.exists() {
        found_count += 1;
    }

    if let Some(manifest) = installer::Installer::read_manifest(&config.game_dir) {
        Ok(format!(
            "Instalación íntegra detectada (Versión {}, Build {}). {} ejecutables verificados.",
            manifest.version, manifest.build_sequence, found_count
        ))
    } else {
        Ok(format!(
            "Archivos principales presentes ({} ejecutables verificados).",
            found_count
        ))
    }
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
async fn pick_path(
    is_directory: bool,
    title: Option<String>,
    default_path: Option<String>,
) -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || {
        let mut cmd =
            if std::path::Path::new("/usr/bin/zenity").exists() || tweaks::which_exists("zenity") {
                let mut c = std::process::Command::new("zenity");
                c.arg("--file-selection");
                if is_directory {
                    c.arg("--directory");
                }
                if let Some(ref t) = title {
                    c.arg(format!("--title={}", t));
                }
                if let Some(ref p) = default_path {
                    if !p.is_empty() {
                        let clean = p.trim_end_matches('/');
                        if is_directory {
                            c.arg(format!("--filename={}/", clean));
                        } else {
                            c.arg(format!("--filename={}", clean));
                        }
                    }
                }
                c
            } else if tweaks::which_exists("kdialog") {
                let mut c = std::process::Command::new("kdialog");
                if is_directory {
                    c.arg("--getexistingdirectory");
                } else {
                    c.arg("--getopenfilename");
                }
                if let Some(ref p) = default_path {
                    if !p.is_empty() {
                        c.arg(p);
                    }
                }
                c
            } else {
                return Err(
                    "No se encontró zenity ni kdialog para abrir el selector nativo.".to_string(),
                );
            };

        match cmd.output() {
            Ok(output) if output.status.success() => {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if path.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(path))
                }
            }
            Ok(_) => Ok(None),
            Err(e) => Err(format!("Error al ejecutar selector de archivos: {}", e)),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

fn validate_external_url(url: &str) -> Result<&str, String> {
    let trimmed = url.trim();
    if !(trimmed.starts_with("https://") || trimmed.starts_with("http://")) {
        return Err("Solo se permiten URLs con esquema http:// o https://".to_string());
    }
    if trimmed.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("La URL contiene caracteres inválidos.".to_string());
    }
    Ok(trimmed)
}

fn read_log_tail(path: &std::path::Path, max_bytes: u64) -> std::io::Result<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let truncated = len > max_bytes;
    if truncated {
        file.seek(SeekFrom::End(-(max_bytes as i64)))?;
    }
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    if truncated {
        Ok(format!(
            "[... mostrando los últimos {} KB de {} KB ...]\n{}",
            max_bytes / 1024,
            len / 1024,
            text
        ))
    } else {
        Ok(text)
    }
}

#[tauri::command]
fn open_external(url: String) -> Result<(), String> {
    let safe_url = validate_external_url(&url)?;
    std::process::Command::new("xdg-open")
        .arg(safe_url)
        .spawn()
        .map_err(|e| format!("No se pudo abrir el enlace externo: {}", e))?;
    Ok(())
}

#[tauri::command]
async fn get_logs() -> Result<String, String> {
    let config = LauncherConfig::load().map_err(|e| e.to_string())?;
    let log_path = config.log_file_path();
    tokio::task::spawn_blocking(move || read_log_tail(&log_path, 256 * 1024))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn clear_logs() -> Result<(), String> {
    let config = LauncherConfig::load().map_err(|e| e.to_string())?;
    std::fs::write(config.log_file_path(), "").map_err(|e| e.to_string())
}

#[tauri::command]
fn cancel_auth_flow(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.auth_cancel.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(notify) = guard.take() {
        notify.notify_waiters();
    }
    Ok(())
}

#[tauri::command]
async fn start_auth_flow(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    channel: String,
    remember: Option<bool>,
) -> Result<String, String> {
    let persist = remember.unwrap_or(true);
    let cancel_notify = Arc::new(tokio::sync::Notify::new());
    {
        let mut guard = state.auth_cancel.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(prev) = guard.replace(cancel_notify.clone()) {
            prev.notify_waiters();
        }
    }

    let (port, rx) = auth::AuthManager::listen_for_auth_callback(55000, cancel_notify, persist)
        .await
        .map_err(|e| e.to_string())?;

    let auth_url = auth::AuthManager::build_auth_url(&channel, port, "es");
    println!("Abriendo Netmarble Members SSO en navegador: {}", auth_url);

    let _ = std::process::Command::new("xdg-open")
        .arg(&auth_url)
        .spawn();

    let app_handle_clone = app_handle.clone();
    let ephemeral_lock = state.ephemeral_session.clone();

    tokio::spawn(async move {
        match rx.await {
            Ok(session) => {
                println!(
                    "✓ Sesión oficial Netmarble NM recibida para {}",
                    session.profile_name
                );
                let status = auth::AuthManager::status_from_session(&session);
                let profile_name = session.profile_name.clone();
                if persist {
                    *ephemeral_lock.lock().unwrap_or_else(|e| e.into_inner()) = None;
                } else {
                    *ephemeral_lock.lock().unwrap_or_else(|e| e.into_inner()) = Some(session);
                }
                let _ = app_handle_clone.emit(
                    "auth-status-changed",
                    serde_json::json!({
                        "success": true,
                        "status": status,
                        "profile_name": profile_name
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
    state: tauri::State<'_, AppState>,
    input: String,
    remember: Option<bool>,
) -> Result<auth::AuthStatusPayload, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("El token, URL o payload no puede estar vacío.".to_string());
    }

    let persist = remember.unwrap_or(true);
    let session = auth::AuthManager::process_auth_result_with_persist(trimmed, persist)
        .await
        .map_err(|e| e.to_string())?;

    let status = auth::AuthManager::status_from_session(&session);
    let profile_name = session.profile_name.clone();
    if persist {
        *state
            .ephemeral_session
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
    } else {
        *state
            .ephemeral_session
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(session);
    }

    let _ = app_handle.emit(
        "auth-status-changed",
        serde_json::json!({
            "success": true,
            "status": status,
            "profile_name": profile_name
        }),
    );

    Ok(status)
}

#[tauri::command]
fn logout(app_handle: tauri::AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    *state
        .ephemeral_session
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = None;
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

const DEFAULT_BG_MANIFEST_JSON: &str = include_str!("../ui/backgrounds.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MediaCacheMeta {
    version: String,
    video_path: Option<String>,
    poster_path: Option<String>,
    slides: Vec<api::BackgroundSlide>,
}

fn extract_video_frame0(video_path: &std::path::Path, poster_path: &std::path::Path) -> bool {
    std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-i"])
        .arg(video_path)
        .args(["-vframes", "1", "-q:v", "2"])
        .arg(poster_path)
        .status()
        .map(|s| s.success() && poster_path.exists())
        .unwrap_or(false)
}

#[tauri::command]
async fn get_background_media(
    version: Option<String>,
) -> Result<api::ResolvedBackgroundMedia, String> {
    let config = LauncherConfig::load().unwrap_or_default();
    let cache_dir = LauncherConfig::app_media_cache_dir();
    let meta_path = cache_dir.join("cache_meta.json");

    let resolved_version = match version.filter(|v| !v.trim().is_empty()) {
        Some(v) => v,
        None => {
            let api_client = api::ApiClient::new();
            api_client
                .fetch_build_info()
                .await
                .map(|b| b.version)
                .unwrap_or_else(|_| "default".to_string())
        }
    };

    // 1. CACHE-FIRST: si ya tenemos esta versión en disco, devolvemos inmediato sin llamar a la API
    if let Ok(raw) = std::fs::read_to_string(&meta_path) {
        if let Ok(cached) = serde_json::from_str::<MediaCacheMeta>(&raw) {
            let video_ok = cached
                .video_path
                .as_ref()
                .map(|p| std::path::Path::new(p).exists())
                .unwrap_or(false);
            if cached.version == resolved_version && video_ok {
                return Ok(api::ResolvedBackgroundMedia {
                    version: cached.version,
                    animated: config.animated_background,
                    video_url: String::new(),
                    poster_url: String::new(),
                    cached_video_path: cached.video_path,
                    cached_poster_path: cached.poster_path,
                    slides: cached.slides,
                });
            }
        }
    }

    // 2. VERSIÓN NUEVA O CACHÉ VACÍA: consultar API oficial una sola vez y guardar localmente
    let api_client = api::ApiClient::new();
    let manifest = if !config.background_manifest_url.trim().is_empty() {
        match api_client
            .fetch_remote_manifest(config.background_manifest_url.trim())
            .await
        {
            Ok(remote) => remote,
            Err(_) => serde_json::from_str(DEFAULT_BG_MANIFEST_JSON).unwrap_or_default(),
        }
    } else {
        serde_json::from_str(DEFAULT_BG_MANIFEST_JSON).unwrap_or_default()
    };

    let mut entry = api::ApiClient::resolve_manifest_entry(&manifest, &resolved_version);

    if config.background_manifest_url.trim().is_empty() {
        let (bg_res, forum_slides) = tokio::join!(
            api_client.fetch_official_launcher_background(),
            api_client.fetch_official_forum_slides(),
        );
        if let Ok((live_video, live_poster)) = bg_res {
            if !live_video.is_empty() {
                entry.video_url = live_video;
            }
            if !live_poster.is_empty() {
                entry.poster_url = live_poster;
            }
        }
        if !forum_slides.is_empty() {
            entry.slides = forum_slides;
        }
    }

    let cached_video_buf = if entry.video_url.starts_with("https://") {
        api_client
            .cache_media_file(
                &entry.video_url,
                &cache_dir,
                &resolved_version,
                "video",
                "webm",
            )
            .await
            .ok()
    } else {
        None
    };

    // Extraer fotograma 0 del video descargado como póster local exacto (cero saltos visuales)
    let cached_poster_buf = if let Some(ref vid) = cached_video_buf {
        let poster_target = vid.with_extension("jpg");
        if poster_target.exists() || extract_video_frame0(vid, &poster_target) {
            Some(poster_target)
        } else {
            None
        }
    } else if entry.poster_url.starts_with("https://") {
        api_client
            .cache_media_file(
                &entry.poster_url,
                &cache_dir,
                &resolved_version,
                "poster",
                "jpg",
            )
            .await
            .ok()
    } else {
        None
    };

    let cached_video_path = cached_video_buf.map(|p| p.to_string_lossy().to_string());
    let cached_poster_path = cached_poster_buf.map(|p| p.to_string_lossy().to_string());

    // Persistir cache_meta.json para futuros arranques instantáneos sin tocar red
    if cached_video_path.is_some() || cached_poster_path.is_some() {
        let _ = std::fs::create_dir_all(&cache_dir);
        let meta = MediaCacheMeta {
            version: resolved_version.clone(),
            video_path: cached_video_path.clone(),
            poster_path: cached_poster_path.clone(),
            slides: entry.slides.clone(),
        };
        if let Ok(json) = serde_json::to_string_pretty(&meta) {
            let _ = std::fs::write(&meta_path, json);
        }
    }

    Ok(api::ResolvedBackgroundMedia {
        version: resolved_version,
        animated: config.animated_background,
        video_url: entry.video_url,
        poster_url: entry.poster_url,
        cached_video_path,
        cached_poster_path,
        slides: entry.slides,
    })
}

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

    if args.download {
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(run_headless_download_and_install(&config))?;
        return Ok(());
    }

    if args.update {
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(run_headless_update(&config))?;
        return Ok(());
    }

    if args.play {
        run_headless_play(&config)?;
        return Ok(());
    }

    // Default: Launch Tauri GUI
    tauri::Builder::default()
        .manage(AppState::new())
        .setup(|app| {
            if tweaks::SystemTweaks::is_running_in_gamescope() {
                use tauri::Manager;
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.set_fullscreen(true);
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_launcher_status,
            get_background_media,
            start_download,
            cancel_download,
            launch_game,
            get_config_and_runners,
            save_config,
            apply_fps_tweak,
            clear_shader_cache,
            rewrite_manifest,
            verify_installed_game,
            pick_path,
            open_folder,
            open_external,
            get_logs,
            clear_logs,
            start_auth_flow,
            cancel_auth_flow,
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

async fn run_headless_update(config: &LauncherConfig) -> Result<()> {
    println!("Comprobando actualizaciones contra Netmarble API...");
    let api = api::ApiClient::new();
    let build = api.fetch_build_info().await?;

    if config.is_installed() {
        if let Some(manifest) = installer::Installer::read_manifest(&config.game_dir) {
            if manifest.build_sequence >= build.build_sequence {
                println!(
                    "✓ El cliente ya está actualizado (Versión {}, Build {}).",
                    manifest.version, manifest.build_sequence
                );
                return Ok(());
            }
            println!(
                "Actualización disponible: Build {} -> {} (v{}).",
                manifest.build_sequence, build.build_sequence, build.version
            );
        }
    }

    run_headless_download_and_install(config).await
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
            "\r[{:3.1}%] {:.1} MB / {:.1} MB | {} | ETA: {} | {}",
            prog.progress_ratio * 100.0,
            prog.downloaded_mb(),
            prog.total_mb(),
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
    )?;

    let _ = std::fs::remove_file(&downloaded_file);
    println!("✓ ¡Instalación completada exitosamente!");
    Ok(())
}

fn run_headless_play(config: &LauncherConfig) -> Result<()> {
    let mut child = runner::GameRunner::launch(config, None)?;
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
        .unwrap_or_else(|| PathBuf::from("."))
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

#[cfg(test)]
mod tests {
    use super::{read_log_tail, validate_external_url};
    use std::fs;

    #[test]
    fn validate_external_url_permite_solo_http_https() {
        assert!(validate_external_url("https://stardive.netmarble.com/").is_ok());
        assert!(validate_external_url("http://127.0.0.1:55000/redirectLauncher").is_ok());
        assert!(validate_external_url("file:///etc/passwd").is_err());
        assert!(validate_external_url("javascript:alert(1)").is_err());
        assert!(validate_external_url("https://example.com/\nwhoami").is_err());
    }

    #[test]
    fn read_log_tail_trunca_archivos_grandes() {
        let tmp =
            std::env::temp_dir().join(format!("stardive_log_tail_{}.log", std::process::id()));
        let content = format!("{}FINAL_MARKER", "A".repeat(4096));
        fs::write(&tmp, &content).unwrap();

        let tail = read_log_tail(&tmp, 1024).unwrap();
        assert!(tail.contains("FINAL_MARKER"));
        assert!(tail.contains("mostrando los últimos"));

        let _ = fs::remove_file(&tmp);
    }
}
