pub mod structs;
pub mod manager;

use crate::{config, APP_CONFIG_DIR};

use log::info;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

pub use manager::SettingsManager;

fn get_db_file_path() -> PathBuf {
    PathBuf::from(format!(
        "{}/{}",
        APP_CONFIG_DIR.get().unwrap().display(),
        config::DB_FILE_NAME
    ))
}

pub fn init_settings() -> structs::Settings {
    let db_file_path = get_db_file_path();

    info!(
        "Loading settings db file located at: {}",
        db_file_path.display()
    );

    if db_file_path.exists() {
        if let Ok(db_file) = File::open(&db_file_path) {
            let reader = BufReader::new(db_file);
            if let Ok(settings) = serde_json::from_reader(reader) {
                info!("Settings loaded.");
                return settings;
            }
        }
        // the file is there but cannot be read: it is kept (renamed), never overwritten by the defaults silently
        let (settings, problem): (structs::Settings, Option<String>) =
            crate::uistate::parse_or_keep(&db_file_path, "основной файл настроек (app.db)");
        if let Some(p) = problem {
            crate::uistate::add_load_problem(&p);
        }
        return settings;
    }

    warn!("No settings file found. Creating default struct.");
    structs::Settings::default()
}

/// init settings and return a SettingsManager ready to use
pub fn init() -> SettingsManager {
    let settings = init_settings();
    SettingsManager::new(settings)
}

pub fn save_settings(settings: &structs::Settings) -> Result<(), std::io::Error> {
    let db_file_path = get_db_file_path();

    crate::write_atomic(
        &db_file_path,
        serde_json::to_string_pretty(&settings).unwrap(),
    )?;

    info!("Settings saved to: {:#}", db_file_path.display());
    Ok(())
}
