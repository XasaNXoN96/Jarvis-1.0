use tray_icon::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};

use jarvis_core::{i18n, SettingsManager};

// RADIO GROUP

// a group of check menu items where only one can be active at a time.
// stores (menu_item, setting_value) pairs.
pub struct RadioGroup {
    pub setting_key: String,
    pub items: Vec<(CheckMenuItem, String)>,
}

impl RadioGroup {
    pub fn select(&self, value: &str) {
        for (item, val) in &self.items {
            item.set_checked(val == value);
        }
    }
}

// TRAY MENU STATE

pub struct TrayMenu {
    pub menu: Menu,
    pub state: TrayState,
}

// holds references to menu items for updating check marks after build
pub struct TrayState {
    pub radio_groups: Vec<RadioGroup>,
    pub gain_toggle: CheckMenuItem,
    pub mic_toggle: CheckMenuItem,
}

// BUILD

// Only what Jarvis needs: microphone switch (voice <-> text mode), the window and exit.
// (Language / voice pack / wake-word engine / noise / VAD submenus of the original project are gone;
// the settings themselves keep working, they are just not shown here.)
pub fn build(settings: &SettingsManager) -> TrayMenu {
    let menu = Menu::new();

    // kept for the TrayState layout, not shown
    let gain_on = settings.read("gain_normalizer").map(|v| v == "true").unwrap_or(true);
    let gain_toggle = CheckMenuItem::with_id("toggle:gain_normalizer", i18n::t("tray-gain-normalizer"), true, gain_on, None);

    // microphone off = text-only mode
    let mic_toggle = CheckMenuItem::with_id(
        "toggle:mic_muted",
        i18n::t("tray-mic-muted"),
        true,
        jarvis_core::MIC_MUTED.load(std::sync::atomic::Ordering::SeqCst),
        None,
    );

    let _ = menu.append(&mic_toggle);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&MenuItem::with_id("open_ui", i18n::t("tray-open-window"), true, None));
    // the own window that can be hidden exists on Windows only
    #[cfg(target_os = "windows")]
    let _ = menu.append(&MenuItem::with_id("hide_ui", i18n::t("tray-hide-window"), true, None));
    let _ = menu.append(&MenuItem::with_id("exit", i18n::t("tray-exit"), true, None));

    TrayMenu {
        menu,
        state: TrayState {
            radio_groups: Vec::new(),
            gain_toggle,
            mic_toggle,
        },
    }
}
