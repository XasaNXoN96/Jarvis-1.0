//! The web camera of this computer, shown on the linked phone («Jarvis Remote», item 40.6 of the plan).
//!
//! What protects it (each point is a test below):
//! - the ability «Показывать веб-камеру этого компьютера на привязанном телефоне» is OFF by default and is switched on only by the owner in the
//!   permission centre; it is asked on EVERY request, so switching it off turns the camera off at once;
//! - the camera is switched on only by an explicit request of the paired phone (`start`), never by itself and never by a command of any other door;
//! - while it runs, a red sign with a «Выключить» button is shown in the window of Jarvis (the page reads `live()`), a notification is sent, and the
//!   journal has the start and the stop. The sign cannot be hidden by the phone;
//! - a session ends by itself after a silence of the phone or after ten minutes; one session at a time;
//! - a camera that is busy (another program uses it) or missing gives a plain message, and the session does not start;
//! - pictures are sent one at a time on request, nothing is recorded or kept on the computer or in the journal.

use crate::live::{End, Live};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

pub const IDLE: Duration = Duration::from_secs(20);
pub const MAX: Duration = Duration::from_secs(10 * 60);
const MIN_GAP: Duration = Duration::from_millis(250);

// what the phone may choose: the width of a picture and the JPEG quality
pub fn quality(name: &str) -> Option<(u32, u8)> {
    match name {
        "low" => Some((320, 50)),
        "medium" | "" => Some((640, 60)),
        "high" => Some((1024, 70)),
        _ => None,
    }
}

// What does the work. The program uses the camera of Windows (below); a test puts its own here.
pub trait Source: Send {
    fn names(&mut self) -> Result<Vec<String>, String>;
    fn open(&mut self, index: usize) -> Result<(), String>;
    fn frame_jpeg(&mut self, width: u32, quality: u8) -> Result<Vec<u8>, String>;
    fn close(&mut self);
}

static LIVE: Lazy<Mutex<Live>> = Lazy::new(|| Mutex::new(Live::new(IDLE, MAX)));
static LAST_FRAME: Lazy<Mutex<Option<Instant>>> = Lazy::new(|| Mutex::new(None));
static CHOICE: Lazy<Mutex<(usize, String)>> = Lazy::new(|| Mutex::new((0, "medium".to_string())));
static SOURCE: Lazy<Mutex<Option<Box<dyn Source>>>> = Lazy::new(|| Mutex::new(None));

fn with_source<R>(f: impl FnOnce(&mut dyn Source) -> R) -> R {
    let mut g = SOURCE.lock();
    if g.is_none() {
        *g = Some(system_source());
    }
    f(g.as_mut().unwrap().as_mut())
}

#[cfg(test)]
pub(crate) fn set_source_for_tests(s: Option<Box<dyn Source>>) {
    *SOURCE.lock() = s;
}

fn allowed() -> Result<(), String> {
    crate::perms::check(crate::perms::Cap::RemoteCamera)
}

// the cameras of the computer (names only): asked by the phone to choose one; needs the ability
pub fn list() -> Result<Vec<String>, String> {
    allowed()?;
    with_source(|s| s.names())
}

// the phone asks to switch the camera on
pub fn start(index: usize, quality_name: &str) -> Result<Value, String> {
    allowed().map_err(|e| {
        crate::audit::record_rate_limited("remote-camera-denied", 60, "remote", "camera_start", "refused", "камера для телефона выключена владельцем");
        e
    })?;
    let (width, q) = quality(quality_name).ok_or("качество: low, medium или high")?;
    let _ = (width, q);
    let running = LIVE.lock().active(Instant::now());
    if running {
        // already on: the same session goes on (a second phone page does not open a second camera)
        LIVE.lock().touch(Instant::now()).ok();
        let idx = CHOICE.lock().0;
        *CHOICE.lock() = (idx, quality_name.to_string());
        return Ok(json!({ "ok": true, "already": true, "message": "Камера уже включена." }));
    }
    let opened = with_source(|s| {
        let names = s.names()?;
        if names.is_empty() {
            return Err("на компьютере не найдена веб-камера".to_string());
        }
        if index >= names.len() {
            return Err("такой камеры нет в списке".to_string());
        }
        s.open(index)?;
        Ok(names[index].clone())
    });
    let name = match opened {
        Ok(n) => n,
        Err(e) => {
            with_source(|s| s.close());
            crate::audit::record_rate_limited("remote-camera-failed", 60, "remote", "camera_start", "refused", &e);
            return Err(e);
        }
    };
    LIVE.lock().start(Instant::now());
    *CHOICE.lock() = (index, quality_name.to_string());
    *LAST_FRAME.lock() = None;
    crate::audit::record_door("phone", "remote", "camera_start", "high", "allowed", true, &format!("телефон включил веб-камеру: {}", name));
    crate::notify::push("important", "Веб-камера включена", &format!("Привязанный телефон смотрит через камеру «{}». Выключить: красная полоса вверху окна Jarvis.", name), false);
    Ok(json!({ "ok": true, "camera": name, "message": "Камера включена. Она выключится сама через 20 секунд без запросов или через 10 минут.", "idle_secs": IDLE.as_secs(), "max_secs": MAX.as_secs() }))
}

pub fn stop(reason: &str) -> bool {
    let was = LIVE.lock().stop();
    with_source(|s| s.close());
    if was {
        crate::audit::record_door("phone", "remote", "camera_stop", "high", "allowed", true, reason);
    }
    was
}

// is the camera on now? (an expired session is closed here: the camera goes off and the sign goes out)
pub fn live() -> bool {
    let r = LIVE.lock().check(Instant::now());
    match r {
        Ok(()) => {
            if allowed().is_err() {
                stop("разрешение выключено владельцем");
                return false;
            }
            true
        }
        Err(End::Stopped) => false,
        Err(e) => {
            with_source(|s| s.close());
            crate::audit::record_door("phone", "remote", "camera_stop", "high", "allowed", true, e.text());
            false
        }
    }
}

// one picture for the phone
pub fn frame() -> Result<Vec<u8>, String> {
    if let Err(e) = allowed() {
        stop("разрешение выключено владельцем");
        return Err(e);
    }
    if let Err(e) = LIVE.lock().touch(Instant::now()) {
        with_source(|s| s.close());
        return Err(if e == End::Stopped { "Камера не включена: нажмите «Включить камеру» на телефоне.".to_string() } else { format!("Камера выключена: {}. Включите её снова.", e.text()) });
    }
    {
        let mut last = LAST_FRAME.lock();
        let now = Instant::now();
        if last.map(|l| now.saturating_duration_since(l) < MIN_GAP).unwrap_or(false) {
            return Err("слишком часто: не больше четырёх кадров в секунду".into());
        }
        *last = Some(now);
    }
    let (_, qname) = CHOICE.lock().clone();
    let (width, q) = quality(&qname).unwrap_or((640, 60));
    let r = with_source(|s| s.frame_jpeg(width, q));
    if let Err(e) = &r {
        // a camera that was taken away or unplugged ends the session: the sign must not say «включена» over a dead camera
        stop(&format!("камера перестала отвечать: {}", e));
    }
    r
}

pub fn status_json() -> Value {
    json!({ "allowed": allowed().is_ok(), "live": live(), "seconds": LIVE.lock().seconds(Instant::now()), "quality": CHOICE.lock().1.clone() })
}

// ---------------------------------------------------------------------------------------------------------------------------------
// the camera of Windows (Media Foundation through nokhwa); other systems have none here

#[cfg(target_os = "windows")]
fn system_source() -> Box<dyn Source> {
    Box::new(win::WinCamera::default())
}

#[cfg(not(target_os = "windows"))]
fn system_source() -> Box<dyn Source> {
    struct None_;
    impl Source for None_ {
        fn names(&mut self) -> Result<Vec<String>, String> {
            Err("веб-камера доступна только в Windows".into())
        }
        fn open(&mut self, _: usize) -> Result<(), String> {
            Err("веб-камера доступна только в Windows".into())
        }
        fn frame_jpeg(&mut self, _: u32, _: u8) -> Result<Vec<u8>, String> {
            Err("веб-камера доступна только в Windows".into())
        }
        fn close(&mut self) {}
    }
    Box::new(None_)
}

#[cfg(target_os = "windows")]
mod win {
    use super::Source;
    use nokhwa::pixel_format::RgbFormat;
    use nokhwa::utils::{ApiBackend, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType};
    use nokhwa::Camera;

    // The camera object of nokhwa is not marked `Send`. It is only ever used while the lock of `SOURCE` is held (one thread at a time),
    // so handing it from one thread to another is what the lock already guarantees. Not verified on real hardware.
    struct Held(Camera);
    unsafe impl Send for Held {}

    #[derive(Default)]
    pub struct WinCamera {
        cam: Option<Held>,
    }

    fn why(e: nokhwa::NokhwaError) -> String {
        let t = e.to_string();
        // the usual reasons, said plainly (the technical text follows)
        format!("камера недоступна: её, вероятно, уже использует другая программа, или её отключили в параметрах конфиденциальности Windows ({})", t.chars().take(120).collect::<String>())
    }

    impl Source for WinCamera {
        fn names(&mut self) -> Result<Vec<String>, String> {
            nokhwa::query(ApiBackend::MediaFoundation)
                .map(|l| l.into_iter().map(|c| c.human_name()).collect())
                .map_err(|e| format!("не удалось получить список камер: {}", e.to_string().chars().take(120).collect::<String>()))
        }

        fn open(&mut self, index: usize) -> Result<(), String> {
            self.close();
            let fmt = RequestedFormat::new::<RgbFormat>(RequestedFormatType::None);
            let mut cam = Camera::with_backend(CameraIndex::Index(index as u32), fmt, ApiBackend::MediaFoundation).map_err(why)?;
            cam.open_stream().map_err(why)?;
            self.cam = Some(Held(cam));
            Ok(())
        }

        fn frame_jpeg(&mut self, width: u32, quality: u8) -> Result<Vec<u8>, String> {
            let cam = &mut self.cam.as_mut().ok_or("камера не открыта")?.0;
            let buf = cam.frame().map_err(why)?;
            // motion JPEG is read by the pure Rust reader of the image crate, the other formats by the decoders of nokhwa
            let rgb = if buf.source_frame_format() == FrameFormat::MJPEG {
                image::load_from_memory_with_format(buf.buffer(), image::ImageFormat::Jpeg).map_err(|e| format!("кадр камеры не прочитан: {}", e))?.to_rgb8()
            } else {
                buf.decode_image::<RgbFormat>().map_err(|e| format!("кадр камеры не прочитан: {}", e))?
            };
            let rgb = if rgb.width() > width {
                let h = ((rgb.height() as u64 * width as u64) / rgb.width() as u64).max(1) as u32;
                image::imageops::resize(&rgb, width, h, image::imageops::FilterType::Triangle)
            } else {
                rgb
            };
            let mut out = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality).encode_image(&rgb).map_err(|e| format!("кадр не сжат: {}", e))?;
            Ok(out)
        }

        fn close(&mut self) {
            if let Some(mut c) = self.cam.take() {
                let _ = c.0.stop_stream();
            }
        }
    }
}

// a camera that shows one grey picture, for the tests of other modules
#[cfg(test)]
pub(crate) struct FakeCam;
#[cfg(test)]
impl Source for FakeCam {
    fn names(&mut self) -> Result<Vec<String>, String> {
        Ok(vec!["Тестовая камера".into()])
    }
    fn open(&mut self, _: usize) -> Result<(), String> {
        Ok(())
    }
    fn frame_jpeg(&mut self, _: u32, _: u8) -> Result<Vec<u8>, String> {
        Ok(b"jpeg".to_vec())
    }
    fn close(&mut self) {}
}

#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = parking_lot::const_mutex(());

#[cfg(test)]
pub(crate) fn reset_for_tests() {
    *LIVE.lock() = Live::new(IDLE, MAX);
    *LAST_FRAME.lock() = None;
    *CHOICE.lock() = (0, "medium".to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Default)]
    struct Fake {
        cameras: Vec<String>,
        opened: Arc<Mutex<Vec<String>>>,
        busy: bool,
        dead: bool,
    }
    impl Source for Fake {
        fn names(&mut self) -> Result<Vec<String>, String> {
            Ok(self.cameras.clone())
        }
        fn open(&mut self, index: usize) -> Result<(), String> {
            if self.busy {
                return Err("камера недоступна: её, вероятно, уже использует другая программа".into());
            }
            self.opened.lock().push(format!("open {}", index));
            Ok(())
        }
        fn frame_jpeg(&mut self, width: u32, quality: u8) -> Result<Vec<u8>, String> {
            if self.dead {
                return Err("камера отключена".into());
            }
            Ok(format!("jpeg {} {}", width, quality).into_bytes())
        }
        fn close(&mut self) {
            self.opened.lock().push("close".into());
        }
    }

    fn setup(f: Fake) -> Arc<Mutex<Vec<String>>> {
        reset_for_tests();
        let log = f.opened.clone();
        set_source_for_tests(Some(Box::new(f)));
        log
    }

    fn two() -> Fake {
        Fake { cameras: vec!["Встроенная камера".into(), "USB камера".into()], ..Default::default() }
    }

    fn pause() {
        std::thread::sleep(MIN_GAP + Duration::from_millis(30));
    }

    #[test]
    fn the_qualities_are_a_short_list() {
        assert_eq!(quality("low"), Some((320, 50)));
        assert_eq!(quality(""), Some((640, 60)));
        assert_eq!(quality("high"), Some((1024, 70)));
        assert_eq!(quality("ultra"), None);
        assert_eq!(quality("4000"), None);
    }

    #[test]
    fn the_camera_is_off_by_default_and_nothing_opens_without_the_owner_and_a_request() {
        let _g = TEST_LOCK.lock();
        crate::store::with_temp(|| {
            crate::owner::reset_for_tests();
            let log = setup(two());
            assert!(!crate::perms::allowed(crate::perms::Cap::RemoteCamera), "OFF by default");
            assert!(list().is_err() && start(0, "medium").is_err() && frame().is_err());
            assert!(log.lock().iter().all(|l| l == "close"), "the camera was never opened: {:?}", log.lock());
            crate::perms::set(crate::perms::Cap::RemoteCamera, true).unwrap();
            // allowed, but not started: no picture
            let e = frame().unwrap_err();
            assert!(e.contains("не включена"), "{}", e);
            assert!(log.lock().iter().all(|l| l == "close"), "no camera was opened: {:?}", log.lock());
            assert_eq!(list().unwrap(), vec!["Встроенная камера".to_string(), "USB камера".to_string()]);
            let r = start(1, "low").unwrap();
            assert_eq!(r["camera"], "USB камера");
            assert!(live());
            pause();
            assert_eq!(frame().unwrap(), b"jpeg 320 50".to_vec(), "the chosen quality is used");
            set_source_for_tests(None);
        });
    }

    #[test]
    fn a_missing_a_wrong_or_a_busy_camera_gives_a_plain_message_and_no_session() {
        let _g = TEST_LOCK.lock();
        crate::store::with_temp(|| {
            crate::owner::reset_for_tests();
            crate::perms::set(crate::perms::Cap::RemoteCamera, true).unwrap();
            setup(Fake::default());
            assert!(start(0, "medium").unwrap_err().contains("не найдена веб-камера"));
            setup(two());
            assert!(start(5, "medium").unwrap_err().contains("такой камеры нет"));
            assert!(start(0, "ultra").is_err());
            setup(Fake { busy: true, ..two() });
            let e = start(0, "medium").unwrap_err();
            assert!(e.contains("другая программа"), "{}", e);
            assert!(!live(), "a camera that did not open starts no session and shows no sign");
            set_source_for_tests(None);
        });
    }

    #[test]
    fn switching_the_ability_off_turns_the_camera_off_at_once_and_it_does_not_come_back() {
        let _g = TEST_LOCK.lock();
        crate::store::with_temp(|| {
            crate::owner::reset_for_tests();
            let log = setup(two());
            crate::perms::set(crate::perms::Cap::RemoteCamera, true).unwrap();
            start(0, "medium").unwrap();
            pause();
            frame().unwrap();
            crate::perms::set(crate::perms::Cap::RemoteCamera, false).unwrap();
            assert!(!live(), "the sign goes out");
            assert!(log.lock().last().unwrap() == "close", "the camera was closed: {:?}", log.lock());
            pause();
            assert!(frame().is_err());
            crate::perms::set(crate::perms::Cap::RemoteCamera, true).unwrap();
            pause();
            assert!(frame().unwrap_err().contains("не включена"), "switching the ability on again does not switch the camera on");
            set_source_for_tests(None);
        });
    }

    #[test]
    fn a_second_start_does_not_open_a_second_camera_and_stop_closes_it() {
        let _g = TEST_LOCK.lock();
        crate::store::with_temp(|| {
            crate::owner::reset_for_tests();
            let log = setup(two());
            crate::perms::set(crate::perms::Cap::RemoteCamera, true).unwrap();
            start(0, "medium").unwrap();
            let again = start(0, "high").unwrap();
            assert_eq!(again["already"], true);
            assert_eq!(log.lock().iter().filter(|l| l.starts_with("open")).count(), 1, "one camera opened once");
            assert!(stop("кнопка «Выключить» на компьютере"));
            assert!(!live());
            assert!(!stop("ещё раз"), "nothing was on");
            assert_eq!(log.lock().last().unwrap(), "close");
            // the journal has the start and the stop (with the name of the camera) and no picture
            let j = serde_json::to_string(&crate::audit::json_filtered("", "", "", "", 0, 0)).unwrap();
            assert!(j.contains("camera_start") && j.contains("camera_stop") && j.contains("Встроенная камера"), "{}", j);
            set_source_for_tests(None);
        });
    }

    #[test]
    fn pictures_come_not_too_often_and_a_dead_camera_ends_the_session() {
        let _g = TEST_LOCK.lock();
        crate::store::with_temp(|| {
            crate::owner::reset_for_tests();
            setup(two());
            crate::perms::set(crate::perms::Cap::RemoteCamera, true).unwrap();
            start(0, "medium").unwrap();
            pause();
            assert!(frame().is_ok());
            assert!(frame().unwrap_err().contains("слишком часто"));
            // the camera is unplugged
            setup(Fake { dead: true, ..two() });
            crate::perms::set(crate::perms::Cap::RemoteCamera, true).unwrap();
            start(0, "medium").unwrap();
            pause();
            assert!(frame().unwrap_err().contains("отключена"));
            assert!(!live(), "the sign must not say «включена» over a dead camera");
            set_source_for_tests(None);
        });
    }
}
