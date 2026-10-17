mod menu;

use tray_icon::{
    menu::MenuEvent,
    TrayIconBuilder,
};
use image;
use std::process::Command;

#[cfg(target_os="windows")]
use winit::platform::windows::EventLoopBuilderExtWindows;

use jarvis_core::{config, i18n, voices, ipc::{self, IpcEvent}, SettingsManager};

const TRAY_ICON_BYTES: &[u8] = include_bytes!("../../../resources/icons/32x32.png");

pub fn init_blocking(settings: SettingsManager) {
    // GTK must be initialized before any tray/menu widget is created
    #[cfg(target_os = "linux")]
    gtk::init().unwrap();

    let icon = load_icon_from_bytes(TRAY_ICON_BYTES);

    // build menu with settings submenus
    let tray_menu = menu::build(&settings);
    let menu::TrayMenu { menu, state: tray_state } = tray_menu;

    // (kept in a cell: the exit removes the icon from the notification area BEFORE the process is ended, see crate::shutdown_and_exit)
    let tray_icon = std::rc::Rc::new(std::cell::RefCell::new(Some(TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        // a left click opens the window (window.rs), the menu is on the right click
        .with_menu_on_left_click(false)
        .with_tooltip(i18n::t("tray-tooltip"))
        .with_icon(icon)
        .build()
        .unwrap())));

    let menu_channel = MenuEvent::receiver();

    #[cfg(target_os = "linux")]
    {
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            if let Ok(event) = menu_channel.try_recv() {
                handle_menu_event(&event, &settings, &tray_state);
            }
            glib::ControlFlow::Continue
        });
        gtk::main();
    }

    #[cfg(target_os = "macos")]
    {
        use winit::event_loop::{EventLoop, ControlFlow};
        let event_loop = EventLoop::new().unwrap();
        event_loop.run(move |_event, elwt| {
            elwt.set_control_flow(ControlFlow::Wait);
            if let Ok(event) = menu_channel.try_recv() {
                handle_menu_event(&event, &settings, &tray_state);
            }
        }).unwrap();
    }

    // Windows: one message loop for the tray and the own window of Jarvis (window.rs)
    #[cfg(target_os = "windows")]
    {
        let _ = &menu_channel; // the window loop receives the menu events through its handler
        let args: Vec<String> = std::env::args().collect();
        let has = |flag: &str| args.iter().any(|a| a == flag);
        let show_at_start = !has("--background") && !has("--no-ui");
        let keep_console = has("--console");
        let hooks = crate::window::Hooks {
            url: jarvis_core::ui::url(),
            data_dir: jarvis_core::APP_CONFIG_DIR.get().map(|d| d.join("webview2")),
            icon_png: TRAY_ICON_BYTES,
            title: "Джарвис".to_string(),
            show_at_start,
            on_menu: Box::new(move |event| handle_menu_event(event, &settings, &tray_state)),
            on_exit: Box::new({
                let tray_icon = tray_icon.clone();
                move || {
                    // Windows keeps the icon of a program that was ended without removing it until the mouse passes over it: it is removed
                    // here, on the thread that made it, and only then the process ends
                    drop(tray_icon.borrow_mut().take());
                    crate::TRAY_GONE.store(true, std::sync::atomic::Ordering::SeqCst);
                    crate::shutdown_and_exit()
                }
            }),
            fallback_open: Box::new(|| jarvis_core::ui::open_browser_window()),
            on_first_show: Box::new(move || {
                if !keep_console {
                    // the window replaces the console
                    unsafe {
                        winapi::um::wincon::FreeConsole();
                    }
                }
            }),
        };
        info!("Tray initialized.");
        crate::window::run(hooks);
    }

    #[cfg(not(target_os = "windows"))]
    info!("Tray initialized.");
}

fn handle_menu_event(event: &MenuEvent, settings: &SettingsManager, tray_state: &menu::TrayState) {
    let id = event.id.0.as_str();

    // -- radio group: "set:key:value"
    if let Some(rest) = id.strip_prefix("set:") {
        if let Some((key, value)) = rest.split_once(':') {
            match settings.write(key, value) {
                Ok(()) => {
                    info!("Tray: {} = {}", key, value);

                    // update check marks in the radio group
                    for group in &tray_state.radio_groups {
                        if group.setting_key == key {
                            group.select(value);
                            break;
                        }
                    }

                    // apply side effects
                    match key {
                        "language" => {
                            i18n::set_language(value);
                        }
                        "assistant_voice" => {
                            voices::set_current_voice(value);
                        }
                        _ => {}
                    }
                }
                Err(e) => {
                    warn!("Tray: failed to set {} = {}: {}", key, value, e);
                }
            }
            return;
        }
    }

    // -- toggle: "toggle:key"
    if let Some(key) = id.strip_prefix("toggle:") {
        match key {
            "mic_muted" => {
                // remembered like the switch of the window (privacy: stays off after a restart)
                let muted = tray_state.mic_toggle.is_checked();
                jarvis_core::uistate::set_mic_muted(muted);
                info!("Tray: microphone muted = {}", muted);
            }
            "gain_normalizer" => {
                // CheckMenuItem auto-toggles on click, just read the new state
                let new_val = tray_state.gain_toggle.is_checked();
                let val_str = if new_val { "true" } else { "false" };
                if let Err(e) = settings.write(key, val_str) {
                    warn!("Tray: failed to toggle {}: {}", key, e);
                    // revert visual state on error
                    tray_state.gain_toggle.set_checked(!new_val);
                } else {
                    info!("Tray: {} = {}", key, val_str);
                }
            }
            _ => {}
        }
        return;
    }

    // -- action items
    match id {
        "open_ui" => {
            jarvis_core::ui::open_window();
        }
        "exit" => crate::shutdown_and_exit(),
        "restart" => {
            info!("Restarting from tray menu...");
            restart_app();
        }
        "settings" => {
            info!("Opening settings from tray menu...");
            open_settings();
        }
        _ => {}
    }
}

// HELPERS

fn load_icon_from_bytes(bytes: &[u8]) -> tray_icon::Icon {
    let image = image::load_from_memory(bytes)
        .expect("Failed to load icon")
        .into_rgba8();
    let (width, height) = image.dimensions();
    let rgba = image.into_raw();
    tray_icon::Icon::from_rgba(rgba, width, height).expect("Failed to create icon")
}

fn restart_app() {
    let exe_path = match std::env::current_exe() {
        Ok(path) => path,
        Err(e) => {
            error!("Failed to get executable path: {}", e);
            return;
        }
    };
    
    // the new copy waits until this one has quit (only one Jarvis runs at a time)
    let mut next = Command::new(&exe_path);
    next.arg("--wait-instance");
    jarvis_core::null_stdio(&mut next);
    match next.spawn() {
        Ok(_) => {
            info!("Spawned new instance, exiting current...");
            // the full exit: the microphone and the ports are released for the new copy. On Windows this runs on the thread of the
            // window: the exit goes through its loop (it removes the tray icon first)
            #[cfg(target_os = "windows")]
            {
                crate::window::send(crate::window::UserEvent::Exit);
                return;
            }
            #[cfg(not(target_os = "windows"))]
            crate::shutdown_and_exit();
        }
        Err(e) => {
            error!("Failed to restart: {}", e);
        }
    }
}

fn open_settings() {
    if ipc::has_clients() {
        info!("GUI is connected, sending reveal event");
        ipc::send(IpcEvent::RevealWindow);
    } else {
        info!("GUI not connected, launching jarvis-gui");
        launch_gui();
    }
}

fn launch_gui() {
    let exe_path = match std::env::current_exe() {
        Ok(path) => path,
        Err(e) => {
            error!("Failed to get executable path: {}", e);
            return;
        }
    };
    
    let gui_path = exe_path.parent()
        .map(|p| p.join(get_gui_executable_name()))
        .unwrap_or_else(|| get_gui_executable_name().into());
    
    info!("Launching GUI: {:?}", gui_path);
    
    let mut gui = Command::new(&gui_path);
    jarvis_core::null_stdio(&mut gui);
    match gui.spawn() {
        Ok(_) => info!("Launched jarvis-gui"),
        Err(e) => error!("Failed to launch jarvis-gui: {}", e),
    }
}

#[cfg(target_os = "windows")]
fn get_gui_executable_name() -> &'static str {
    "jarvis-gui.exe"
}

#[cfg(not(target_os = "windows"))]
fn get_gui_executable_name() -> &'static str {
    "jarvis-gui"
}
