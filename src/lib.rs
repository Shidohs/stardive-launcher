mod downloader;
mod launcher;

use tauri::Manager;

#[tauri::command]
fn is_running_in_gamescope() -> bool {
    std::env::var("GAMESCOPE_WAYLAND_DISPLAY").is_ok()
        || std::env::var("SteamDeck").as_deref() == Ok("1")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // Si corre en Steam Deck / Gamescope, maximizar automáticamente
            if is_running_in_gamescope() {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window. set_fullscreen(true);
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            launcher::apply_ue5_tweaks,
            launcher::launch_game,
            downloader::download_with_resume,
            downloader::verify_game_files,
            is_running_in_gamescope
        ])
        .run(tauri::generate_context!())
        .expect("Error al ejecutar stardive-launcher");
}