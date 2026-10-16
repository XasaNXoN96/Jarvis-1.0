// The window of Jarvis on Windows: the same local page (127.0.0.1:9714), shown in a WebView2 window that belongs
// to Jarvis instead of a separate Edge window. So the cross of the window can ask what to do (minimize to the tray,
// exit Jarvis completely, cancel), the tray can hide and show the window, and a second start shows this window
// instead of opening another one. Without the WebView2 runtime the old Edge/Chrome app window is used.
//
// Three states: the window is open; it is hidden in the tray (Jarvis runs, the wake word is listened to when the
// voice is on); Jarvis is closed (the microphone is released, the process ends).

use std::time::{Duration, Instant};

use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::windows::EventLoopBuilderExtWindows;
use tao::window::{Icon, Window, WindowBuilder};
use tray_icon::menu::MenuEvent;
use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};
use wry::{WebContext, WebView, WebViewBuilder};

#[derive(Debug)]
pub enum UserEvent {
    Show,
    Hide,
    Exit,
    // the answer of the page to the cross: "ack" (the dialog is shown), "tray", "exit", "cancel"
    Close(String),
    // the button / F11 of the page: the window covers the whole screen without its frame, or comes back
    Fullscreen(bool),
    // Alt+Enter: the window covers the whole screen without its frame <-> the window with its title bar
    ToggleWindowFullscreen,
    // the button of the page: minimise the window
    Minimize,
    Menu(MenuEvent),
}

static PROXY: OnceCell<Mutex<EventLoopProxy<UserEvent>>> = OnceCell::new();
// asked before the window loop runs (a second start while Jarvis is still starting): "show" / "hide" is remembered
// and done when the loop starts, instead of opening a browser window next to the own one
static EARLY: Mutex<Option<bool>> = parking_lot::const_mutex(None);

pub fn send(e: UserEvent) -> bool {
    let mut early = EARLY.lock();
    if let Some(p) = PROXY.get() {
        return p.lock().send_event(e).is_ok();
    }
    match e {
        UserEvent::Show => *early = Some(true),
        UserEvent::Hide => *early = Some(false),
        _ => return false,
    }
    true
}

// what the window does at start: the start flag, changed by a show/hide asked before the loop ran
pub fn start_visible(show_at_start: bool, early: Option<bool>) -> bool {
    early.unwrap_or(show_at_start)
}

use jarvis_core::ui::{close_action, CloseAction};

// the page did not show its dialog in time (it is not loaded, or it hangs): the window goes to the tray, Jarvis keeps
// running and can be closed from the tray menu — never a silent exit, never a window that cannot be closed
pub const CLOSE_ANSWER_TIMEOUT: Duration = Duration::from_millis(2500);

pub struct Hooks {
    pub url: String,
    pub data_dir: Option<std::path::PathBuf>,
    pub icon_png: &'static [u8],
    pub title: String,
    pub show_at_start: bool,
    // the tray menu items (open / hide / exit are handled here; the rest by the tray module)
    pub on_menu: Box<dyn Fn(&MenuEvent)>,
    // full exit: stops the wake word, releases the microphone, saves and ends the process
    pub on_exit: Box<dyn Fn()>,
    // the WebView2 window could not be made: the old browser window
    pub fallback_open: Box<dyn Fn() -> bool>,
    // the window was shown (the console is not needed any more)
    pub on_first_show: Box<dyn Fn()>,
}

struct Win {
    window: Window,
    webview: WebView,
}

fn load_icon(png: &[u8]) -> Option<Icon> {
    let img = image::load_from_memory(png).ok()?.into_rgba8();
    let (w, h) = img.dimensions();
    Icon::from_rgba(img.into_raw(), w, h).ok()
}

fn open_in_browser(url: &str) {
    // an external link of the page (a found site, a link of an answer) opens in the browser, not in the Jarvis window
    let l = url.to_lowercase();
    if l.starts_with("http://") || l.starts_with("https://") {
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            let mut c = std::process::Command::new("rundll32");
            c.args(["url.dll,FileProtocolHandler", url]).creation_flags(0x0800_0000);
            jarvis_core::null_stdio(&mut c);
            let _ = c.spawn();
        }
    }
}

fn make_window(target: &tao::event_loop::EventLoopWindowTarget<UserEvent>, h: &Hooks, ctx: &mut WebContext, proxy: EventLoopProxy<UserEvent>) -> Result<Win, String> {
    let window = WindowBuilder::new()
        .with_title(&h.title)
        .with_inner_size(tao::dpi::LogicalSize::new(1040.0, 740.0))
        .with_min_inner_size(tao::dpi::LogicalSize::new(380.0, 480.0))
        .with_window_icon(load_icon(h.icon_png))
        // the title bar of the usual window is dark too (the program is dark)
        .with_theme(Some(tao::window::Theme::Dark))
        // the window starts over the whole screen without the title bar (Alt+Enter brings the frame back; the choice is kept)
        .with_fullscreen(if jarvis_core::uistate::window_fullscreen() { Some(tao::window::Fullscreen::Borderless(None)) } else { None })
        .with_visible(false)
        .build(target)
        .map_err(|e| e.to_string())?;
    let local = h.url.clone();
    let ipc_proxy = proxy.clone();
    let builder = WebViewBuilder::new_with_web_context(ctx)
        .with_url(&h.url)
        .with_ipc_handler(move |req| {
            // the close dialog ("close:<answer>") and the full screen button ("full:on" / "full:off") talk to the window
            let body = req.body();
            if let Some(answer) = body.strip_prefix("close:") {
                let _ = ipc_proxy.send_event(UserEvent::Close(answer.to_string()));
            } else if let Some(on) = full_request(body) {
                let _ = ipc_proxy.send_event(UserEvent::Fullscreen(on));
            } else if is_window_toggle(body) {
                let _ = ipc_proxy.send_event(UserEvent::ToggleWindowFullscreen);
            } else if body == "win:min" {
                let _ = ipc_proxy.send_event(UserEvent::Minimize);
            }
        })
        // the page of Jarvis stays in the window; any other address opens in the browser
        .with_navigation_handler(move |url| {
            // the page itself, and the files it makes for saving (the export of the scenarios: blob: / data:)
            if url.starts_with(&local) || url.starts_with("about:") || url.starts_with("blob:") || url.starts_with("data:") {
                true
            } else {
                open_in_browser(&url);
                false
            }
        })
        .with_new_window_req_handler(|url, _| {
            open_in_browser(&url);
            wry::NewWindowResponse::Deny
        });
    let webview = with_debug_port(builder).build(&window).map_err(|e| e.to_string())?;
    Ok(Win { window, webview })
}

// The debugging port of the window: only when JARVIS_WEBVIEW_DEBUG_PORT is set in the environment of this run (the CI of
// the installer uses it to check the interface in the real window). Never set by the program itself or the installer.
fn with_debug_port(builder: WebViewBuilder) -> WebViewBuilder {
    use wry::WebViewBuilderExtWindows;
    match std::env::var("JARVIS_WEBVIEW_DEBUG_PORT").ok().and_then(|p| p.parse::<u16>().ok()).filter(|p| *p >= 1024) {
        // (the default arguments of wry are repeated: giving any replaces them)
        Some(port) => builder.with_additional_browser_args(format!("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --remote-debugging-port={} --remote-debugging-address=127.0.0.1", port)),
        None => builder,
    }
}

// "full:on" / "full:off" of the page; anything else is not a full screen request
pub fn full_request(body: &str) -> Option<bool> {
    match body {
        "full:on" => Some(true),
        "full:off" => Some(false),
        _ => None,
    }
}

// "winfull:toggle" of the page (Alt+Enter)
pub fn is_window_toggle(body: &str) -> bool {
    body == "winfull:toggle"
}

fn show(w: &Win) {
    w.window.set_visible(true);
    w.window.set_minimized(false);
    // the page knows it is on the screen again (it polls the state and notifications count it as seen)
    let _ = w.webview.set_visible(true);
    w.window.set_focus();
}

// hidden in the tray: the page is told it is not visible (document.hidden), so notifications are shown and the
// page stops polling at full speed
fn hide(w: &Win) {
    let _ = w.webview.set_visible(false);
    w.window.set_visible(false);
}

// "Свернуть в трей": the own window is hidden; without it (WebView2 is not available) the browser window is asked to
// close itself — Jarvis keeps running in the tray in both cases
fn hide_any(win: &Option<Win>) {
    match win {
        Some(w) => hide(w),
        None => jarvis_core::ui::request_browser_close(),
    }
}

// Runs the window and the tray messages until Jarvis exits. It is called on a thread of its own (main.rs), not on the
// main thread, whose COM mode other libraries have set already (Tao would panic with RPC_E_CHANGED_MODE), so Tao is
// told that this is not the main thread.
pub fn run(h: Hooks) -> ! {
    let event_loop = {
        let mut builder = EventLoopBuilder::<UserEvent>::with_user_event();
        builder.with_any_thread(true);
        builder.build()
    };
    let proxy = event_loop.create_proxy();
    let early = {
        // (under the lock: a request is either remembered before this point or sent to the loop after it)
        let mut early = EARLY.lock();
        let _ = PROXY.set(Mutex::new(proxy.clone()));
        early.take()
    };

    let menu_proxy = proxy.clone();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        let _ = menu_proxy.send_event(UserEvent::Menu(e));
    }));
    let tray_proxy = proxy.clone();
    TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
        // a click on the tray icon opens the window
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
            let _ = tray_proxy.send_event(UserEvent::Show);
        }
    }));

    let mut ctx = WebContext::new(h.data_dir.clone());
    let mut win: Option<Win> = None;
    let mut first_show_done = false;
    // the cross was pressed and the page has not answered yet
    let mut close_asked: Option<Instant> = None;
    let start_shown = start_visible(h.show_at_start, early);

    event_loop.run(move |event, target, control_flow| {
        *control_flow = match close_asked {
            Some(at) => ControlFlow::WaitUntil(at + CLOSE_ANSWER_TIMEOUT),
            None => ControlFlow::Wait,
        };
        let mut want_show = false;
        match event {
            Event::NewEvents(StartCause::Init) => {
                want_show = start_shown;
            }
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                if let Some(at) = close_asked {
                    if at.elapsed() >= CLOSE_ANSWER_TIMEOUT {
                        close_asked = None;
                        if let Some(w) = &win {
                            log::warn!("Window: the page did not answer the close request, the window goes to the tray");
                            hide(w);
                        }
                    }
                }
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                if let Some(w) = &win {
                    // the page shows the dialog in the language of Jarvis and answers through window.ipc
                    let asked = w.webview.evaluate_script("window.jarvisCloseRequest ? window.jarvisCloseRequest() : window.ipc.postMessage('close:none')").is_ok();
                    close_asked = Some(Instant::now());
                    if !asked {
                        close_asked = None;
                        hide(w);
                    }
                    *control_flow = ControlFlow::WaitUntil(Instant::now() + CLOSE_ANSWER_TIMEOUT);
                }
            }
            Event::UserEvent(UserEvent::Close(answer)) => match close_action(&answer) {
                Some(CloseAction::Ack) => close_asked = None,
                Some(CloseAction::Hide) => {
                    close_asked = None;
                    if let Some(w) = &win {
                        hide(w);
                    }
                    log::info!("Window: minimized to the tray, Jarvis keeps running");
                }
                Some(CloseAction::Exit) => {
                    close_asked = None;
                    if let Some(w) = &win {
                        hide(w);
                    }
                    (h.on_exit)();
                    *control_flow = ControlFlow::Exit;
                }
                Some(CloseAction::Stay) => close_asked = None,
                None => {
                    // a page without the dialog (an old page): the window goes to the tray, as the timeout would do
                    close_asked = None;
                    if let Some(w) = &win {
                        hide(w);
                    }
                }
            },
            Event::UserEvent(UserEvent::Show) => {
                // an old close question must not hide the window that was just opened
                close_asked = None;
                want_show = true;
            }
            Event::UserEvent(UserEvent::Hide) => hide_any(&win),
            Event::UserEvent(UserEvent::Fullscreen(on)) => {
                if let Some(w) = &win {
                    // leaving the full screen mode of the page returns to the choice of the user (the whole screen or the frame)
                    let keep = on || jarvis_core::uistate::window_fullscreen();
                    w.window.set_fullscreen(if keep { Some(tao::window::Fullscreen::Borderless(None)) } else { None });
                }
            }
            Event::UserEvent(UserEvent::Minimize) => {
                if let Some(w) = &win {
                    w.window.set_minimized(true);
                }
            }
            Event::UserEvent(UserEvent::ToggleWindowFullscreen) => {
                if let Some(w) = &win {
                    let now_full = w.window.fullscreen().is_some();
                    w.window.set_fullscreen(if now_full { None } else { Some(tao::window::Fullscreen::Borderless(None)) });
                    jarvis_core::uistate::set_window_fullscreen(!now_full);
                }
            }
            // the Windows session ends or the installer asks Jarvis to close (Restart Manager): a full, clean exit
            Event::LoopDestroyed => {
                (h.on_exit)();
            }
            Event::UserEvent(UserEvent::Exit) => {
                (h.on_exit)();
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(UserEvent::Menu(e)) => match e.id.0.as_str() {
                "open_ui" => {
                    close_asked = None;
                    want_show = true;
                }
                "hide_ui" => hide_any(&win),
                // "Выход" of the tray: the user chose to exit, no question again
                "exit" => {
                    (h.on_exit)();
                    *control_flow = ControlFlow::Exit;
                }
                _ => (h.on_menu)(&e),
            },
            _ => {}
        }
        if want_show {
            // (a failure is tried again next time: after a restart the old WebView2 may still hold its folder)
            // (a browser window of the fallback already open: no own window next to it, no second browser window)
            let browser_open = win.is_none() && jarvis_core::ui::browser_page_alive();
            if win.is_none() && !browser_open {
                match make_window(target, &h, &mut ctx, proxy.clone()) {
                    Ok(w) => {
                        log::info!("Window: the WebView2 window is created");
                        win = Some(w);
                    }
                    Err(e) => log::warn!("Window: WebView2 is not available ({}), the browser window is used", e),
                }
            }
            match &win {
                Some(w) => show(w),
                None if browser_open => log::info!("Window: the browser window is already open"),
                None => {
                    (h.fallback_open)();
                }
            }
            if !first_show_done {
                first_show_done = true;
                (h.on_first_show)();
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_show_or_hide_asked_before_the_loop_decides_the_start() {
        assert!(start_visible(true, None));
        assert!(!start_visible(false, None), "--background stays hidden");
        assert!(start_visible(false, Some(true)), "a second start during the start shows the window");
        assert!(!start_visible(true, Some(false)), "hidden to the tray during the start stays hidden");
    }

    #[test]
    fn only_the_two_full_screen_words_change_the_window() {
        assert_eq!(full_request("full:on"), Some(true));
        assert_eq!(full_request("full:off"), Some(false));
        assert_eq!(full_request("full:yes"), None);
        assert_eq!(full_request("close:tray"), None);
    }

    #[test]
    fn the_window_toggle_is_its_own_word() {
        assert!(is_window_toggle("winfull:toggle"));
        assert!(!is_window_toggle("full:on") && !is_window_toggle("close:exit") && !is_window_toggle("winfull:"));
        assert_eq!(full_request("winfull:toggle"), None, "it is not the full screen mode of the page");
    }
}
