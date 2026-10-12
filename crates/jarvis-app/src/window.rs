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
    Menu(MenuEvent),
}

static PROXY: OnceCell<Mutex<EventLoopProxy<UserEvent>>> = OnceCell::new();

pub fn send(e: UserEvent) -> bool {
    PROXY.get().map(|p| p.lock().send_event(e).is_ok()).unwrap_or(false)
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
            let _ = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).creation_flags(0x0800_0000).spawn();
        }
    }
}

fn make_window(target: &tao::event_loop::EventLoopWindowTarget<UserEvent>, h: &Hooks, ctx: &mut WebContext, proxy: EventLoopProxy<UserEvent>) -> Result<Win, String> {
    let window = WindowBuilder::new()
        .with_title(&h.title)
        .with_inner_size(tao::dpi::LogicalSize::new(1040.0, 740.0))
        .with_min_inner_size(tao::dpi::LogicalSize::new(380.0, 480.0))
        .with_window_icon(load_icon(h.icon_png))
        .with_visible(false)
        .build(target)
        .map_err(|e| e.to_string())?;
    let local = h.url.clone();
    let ipc_proxy = proxy.clone();
    let webview = WebViewBuilder::new_with_web_context(ctx)
        .with_url(&h.url)
        .with_ipc_handler(move |req| {
            // only the close dialog talks to the window: "close:<answer>"
            if let Some(answer) = req.body().strip_prefix("close:") {
                let _ = ipc_proxy.send_event(UserEvent::Close(answer.to_string()));
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
        })
        .build(&window)
        .map_err(|e| e.to_string())?;
    Ok(Win { window, webview })
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

// Runs the window and the tray messages on the main thread until Jarvis exits.
pub fn run(h: Hooks) -> ! {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let _ = PROXY.set(Mutex::new(proxy.clone()));

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
    let start_shown = h.show_at_start;

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
            Event::UserEvent(UserEvent::Hide) => {
                if let Some(w) = &win {
                    hide(w);
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
                "hide_ui" => {
                    if let Some(w) = &win {
                        hide(w);
                    }
                }
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
            if win.is_none() {
                match make_window(target, &h, &mut ctx, proxy.clone()) {
                    Ok(w) => win = Some(w),
                    Err(e) => log::warn!("Window: WebView2 is not available ({}), the browser window is used", e),
                }
            }
            match &win {
                Some(w) => show(w),
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
