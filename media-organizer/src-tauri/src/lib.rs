//! Cinefold - a media library organiser.
//!
//! The pipeline is: scan -> parse -> identify -> match subtitles -> plan ->
//! execute -> decorate. Planning never touches the disk, and every executed
//! operation is journalled so it can be undone.

mod catalog;
mod commands;
mod folder_icon;
mod fsutil;
mod organiser;
mod parser;
mod scanner;
mod settings;
mod subdl;
mod subtitles;
mod tmdb;
mod undo;

use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            let cache_dir = app.path().app_cache_dir()?;
            std::fs::create_dir_all(&config_dir)?;
            std::fs::create_dir_all(&cache_dir)?;

            app.manage(commands::AppState::new(config_dir, cache_dir));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::validate_api_key,
            commands::validate_subdl_key,
            commands::catalog_fetch_subtitles,
            commands::scan_folder,
            commands::get_items,
            commands::include_skipped,
            commands::search_titles,
            commands::apply_match,
            commands::approve_items,
            commands::fetch_item_subtitle,
            commands::set_item_status,
            commands::set_item_episode,
            commands::build_plan,
            commands::execute_plan,
            commands::list_undo_runs,
            commands::undo_run,
            commands::refresh_icon_cache,
            commands::preview_icon,
            commands::reapply_library_icons,
            commands::catalog_list,
            commands::catalog_update,
            commands::catalog_remove,
            commands::catalog_import_library,
            commands::app_paths,
        ])
        .run(tauri::generate_context!())
        .expect("Cinefold failed to start");
}
