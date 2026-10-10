use crate::AppState;

#[tauri::command]
pub fn db_read(state: tauri::State<'_, AppState>, key: &str) -> String {
    state.settings.read(key).unwrap_or_default()
}

#[tauri::command]
pub fn db_write(state: tauri::State<'_, AppState>, key: &str, val: &str) -> bool {
    match state.settings.write(key, val) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("db_write('{}', '{}'): {}", key, val, e);
            false
        }
    }
}
