// «Мои устройства»: one list of the devices Jarvis really knows about, built from what the program already has — this
// computer (health, battery), the phone of the remote (what its page reports), the microphones Windows shows. Nothing is
// invented: a device that has not reported is not in the list, a value the device does not send is "нет данных", a
// function that does not exist is "не поддерживается" or "в разработке" and never pretends to work.
//
// The kinds (PC, laptop, phone, tablet, audio, other) are in the model so that a second computer or a tablet can join
// through the same agent/protocol later; today only the kinds that really report are listed.
// «Привязано» (the device was seen) and «в сети» (it answers now) are different things and are kept apart.
// Nothing here carries a secret: no token, no link, no key.

use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Pc,
    Laptop,
    Phone,
    Tablet,
    Audio,
    Other,
}

impl Kind {
    pub fn key(self) -> &'static str {
        match self {
            Kind::Pc => "pc",
            Kind::Laptop => "laptop",
            Kind::Phone => "phone",
            Kind::Tablet => "tablet",
            Kind::Audio => "audio",
            Kind::Other => "other",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Kind::Pc => "Компьютер",
            Kind::Laptop => "Ноутбук",
            Kind::Phone => "Телефон",
            Kind::Tablet => "Планшет",
            Kind::Audio => "Аудиоустройство",
            Kind::Other => "Устройство",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Online,
    Offline,
    Connecting,
    Error,
}

impl Status {
    fn key(self) -> &'static str {
        match self {
            Status::Online => "online",
            Status::Offline => "offline",
            Status::Connecting => "connecting",
            Status::Error => "error",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Status::Online => "в сети",
            Status::Offline => "не в сети",
            Status::Connecting => "подключение",
            Status::Error => "ошибка",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cap {
    Available,
    NeedsPermission,
    Unavailable,
    NotSupported,
    InDevelopment,
    NoData,
}

impl Cap {
    fn key(self) -> &'static str {
        match self {
            Cap::Available => "available",
            Cap::NeedsPermission => "needs_permission",
            Cap::Unavailable => "unavailable",
            Cap::NotSupported => "not_supported",
            Cap::InDevelopment => "in_development",
            Cap::NoData => "no_data",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Cap::Available => "доступно",
            Cap::NeedsPermission => "нужно разрешение",
            Cap::Unavailable => "сейчас недоступно",
            Cap::NotSupported => "не поддерживается",
            Cap::InDevelopment => "в разработке",
            Cap::NoData => "нет данных",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Capability {
    pub label: &'static str,
    pub state: Cap,
    pub note: String,
}

#[derive(Debug, Clone)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub status: Status,
    pub paired: bool,
    pub last_seen: Option<u64>,
    pub user: Option<String>,
    pub battery: Option<u8>,
    pub charging: Option<bool>,
    pub detail: String,
    // the place the device told (latitude, longitude, accuracy in metres, when) and the network it reports; empty = nothing known
    pub location: Option<(f64, f64, f64, u64)>,
    pub network: String,
    pub caps: Vec<Capability>,
    // the page where the device is managed (an existing one): "system-remote", "settings", "system"
    pub manage: &'static str,
}

fn cap(label: &'static str, state: Cap, note: &str) -> Capability {
    Capability { label, state, note: note.to_string() }
}

// what the list is built from (the real values are gathered by `gather`; a test names its own)
#[derive(Debug, Clone, Default)]
pub struct Inputs {
    pub now: u64,
    pub pc_name: String,
    pub user: String,
    pub battery: Option<(u8, bool)>,
    pub screen_allowed: bool,
    pub remote_on: bool,
    pub phone_last_seen: Option<u64>,
    pub phone_online: bool,
    pub phone_battery: Option<u8>,
    pub phone_charging: Option<bool>,
    pub phone_location: Option<(f64, f64, f64, u64)>,
    pub phone_net: String,
    // is the internet reachable from this computer (None = not checked yet)
    pub pc_internet: Option<bool>,
    pub mics: Vec<String>,
    pub mic_selected: i32,
}

pub fn build(i: &Inputs) -> Vec<Device> {
    let mut out = Vec::new();

    // this computer: always there (it is the one that runs Jarvis)
    let mut pc_caps = vec![
        cap("Состояние компьютера", Cap::Available, "процессор, память, диск, температура — если их отдаёт оборудование"),
        cap("Запуск программ и сайтов", Cap::Available, "командами Jarvis, по правилам безопасности"),
        cap("Файлы и папки", Cap::Available, "поиск, чтение и запись только в разрешённых папках"),
        cap("Диктовка в активное приложение", if cfg!(target_os = "windows") { Cap::Available } else { Cap::NotSupported }, ""),
        cap("Микрофон", if i.mics.is_empty() { Cap::Unavailable } else { Cap::Available }, if i.mics.is_empty() { "Windows не показывает устройств записи" } else { "" }),
        cap("Снимок экрана для ИИ", if i.screen_allowed { Cap::Available } else { Cap::NeedsPermission }, if i.screen_allowed { "только по вашему запросу" } else { "выключено в настройках приватности" }),
        cap("Удалённый доступ к камере, микрофону и экрану", Cap::NotSupported, "скрытого наблюдения нет и не будет"),
    ];
    if i.battery.is_none() {
        pc_caps.push(cap("Заряд батареи", Cap::NoData, "батареи нет (стационарный компьютер) или система её не показывает"));
    }
    out.push(Device {
        id: "this-pc".into(),
        name: if i.pc_name.is_empty() { "Этот компьютер".into() } else { i.pc_name.clone() },
        kind: if i.battery.is_some() { Kind::Laptop } else { Kind::Pc },
        status: Status::Online,
        paired: true,
        last_seen: Some(i.now),
        user: if i.user.is_empty() { None } else { Some(i.user.clone()) },
        battery: i.battery.map(|b| b.0),
        charging: i.battery.map(|b| b.1),
        detail: "Здесь запущен Jarvis (центральное приложение)".into(),
        location: None,
        network: match i.pc_internet { Some(true) => "интернет есть".into(), Some(false) => "нет интернета".into(), None => String::new() },
        caps: pc_caps,
        manage: "system",
    });

    // the phone of the remote: only after its page has reported at least once (nothing is listed for a phone that never came)
    if let Some(seen) = i.phone_last_seen {
        let age = i.now.saturating_sub(seen);
        let (status, detail) = if !i.remote_on {
            (Status::Offline, "Пульт выключен: телефон не может подключиться".to_string())
        } else if i.phone_online {
            (Status::Online, "Телефон на связи".to_string())
        } else if age <= 30 {
            (Status::Connecting, "Связь прервалась, идёт повторное подключение".to_string())
        } else {
            (Status::Offline, "Нет связи: откройте пульт на телефоне".to_string())
        };
        let live = status == Status::Online;
        out.push(Device {
            id: "phone".into(),
            name: "Телефон (пульт)".into(),
            kind: Kind::Phone,
            status,
            paired: true,
            last_seen: Some(seen),
            user: None,
            battery: if live { i.phone_battery } else { None },
            charging: if live { i.phone_charging } else { None },
            detail,
            location: if live { i.phone_location } else { None },
            network: if live { i.phone_net.clone() } else { String::new() },
            caps: vec![
                cap("Команды с телефона", if live { Cap::Available } else { Cap::Unavailable }, "через пульт, с секретной ссылкой"),
                cap("Найти телефон (звуковой сигнал)", if live { Cap::Available } else { Cap::Unavailable }, "работает, пока страница пульта открыта на телефоне"),
                cap("Ссылка или текст на телефон", if live { Cap::Available } else { Cap::Unavailable }, ""),
                cap("Заряд батареи", if live && i.phone_battery.is_some() { Cap::Available } else { Cap::NoData }, if live && i.phone_battery.is_none() { "браузер телефона не сообщает" } else { "" }),
                cap("Местоположение телефона", if live && i.phone_location.is_some() { Cap::Available } else { Cap::NeedsPermission }, if live && i.phone_location.is_some() { "на карте ниже" } else { "включите «Показывать, где телефон» на странице пульта (нужна защищённая ссылка https)" }),
                cap("Камера, микрофон и экран телефона", Cap::NotSupported, "удалённого доступа нет"),
            ],
            manage: "system-remote",
        });
    }

    // the microphones Windows shows (what is there, which one is chosen; nothing about a battery the system does not give)
    for (n, name) in i.mics.iter().enumerate() {
        let chosen = i.mic_selected == n as i32;
        out.push(Device {
            id: format!("audio-in-{}", n),
            name: name.clone(),
            kind: Kind::Audio,
            status: Status::Online,
            paired: true,
            last_seen: Some(i.now),
            user: None,
            battery: None,
            charging: None,
            location: None,
            network: String::new(),
            detail: if chosen { "Выбран для ввода звука".into() } else if i.mic_selected < 0 && n == 0 { "Ввод звука: системное устройство по умолчанию".into() } else { "Можно выбрать для ввода звука в настройках".into() },
            caps: vec![
                cap("Ввод звука (микрофон)", Cap::Available, if chosen { "выбран" } else { "" }),
                cap("Заряд батареи", Cap::NoData, "система не сообщает"),
                cap("Удалённое управление", Cap::NotSupported, ""),
            ],
            manage: "settings",
        });
    }
    out
}

pub fn to_json(list: &[Device]) -> Value {
    json!(list
        .iter()
        .map(|d| json!({
            "id": d.id,
            "name": d.name,
            "kind": d.kind.key(),
            "kind_label": d.kind.label(),
            "status": d.status.key(),
            "status_label": d.status.label(),
            "paired": d.paired,
            "online": d.status == Status::Online,
            "last_seen": d.last_seen,
            "user": d.user,
            "battery": d.battery,
            "charging": d.charging,
            "detail": d.detail,
            "network": d.network,
            "location": d.location.map(|(lat, lon, acc, at)| json!({ "lat": lat, "lon": lon, "accuracy": acc, "at": at })),
            "manage": d.manage,
            "caps": d.caps.iter().map(|c| json!({ "label": c.label, "state": c.state.key(), "state_label": c.state.label(), "note": c.note })).collect::<Vec<_>>(),
        }))
        .collect::<Vec<_>>())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// the real values of this computer, this run
pub fn gather() -> Inputs {
    let phone = crate::remote::phone_json();
    let mics = crate::recorder::get_audio_devices();
    Inputs {
        now: now_secs(),
        pc_name: std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_default(),
        user: std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default(),
        battery: crate::health::battery().map(|b| (b.percent, b.charging)),
        screen_allowed: crate::uistate::settings().ai_screen,
        remote_on: phone["enabled"].as_bool().unwrap_or(false),
        phone_last_seen: phone["last_seen"].as_u64(),
        phone_online: phone["online"].as_bool().unwrap_or(false),
        phone_battery: phone["battery"].as_u64().map(|b| b.min(100) as u8),
        phone_charging: phone["charging"].as_bool(),
        phone_location: phone["location"].as_object().and_then(|l| Some((l.get("lat")?.as_f64()?, l.get("lon")?.as_f64()?, l.get("accuracy")?.as_f64().unwrap_or(0.0), l.get("at")?.as_u64()?))),
        phone_net: phone["net"].as_str().unwrap_or("").to_string(),
        pc_internet: internet_now(),
        mics,
        mic_selected: crate::DB.get().map(|d| d.read().microphone).unwrap_or(-1),
    }
}

// is the internet there: one short connection to a public address, checked at most every 15 seconds in the background (the page never waits)
fn internet_now() -> Option<bool> {
    static LAST: once_cell::sync::Lazy<parking_lot::Mutex<(Option<std::time::Instant>, Option<bool>)>> = once_cell::sync::Lazy::new(|| parking_lot::Mutex::new((None, None)));
    let mut g = LAST.lock();
    let stale = g.0.map(|t| t.elapsed() > std::time::Duration::from_secs(15)).unwrap_or(true);
    if stale {
        g.0 = Some(std::time::Instant::now());
        std::thread::spawn(|| {
            use std::net::ToSocketAddrs;
            let ok = "1.1.1.1:443".to_socket_addrs().ok().and_then(|mut a| a.next()).map(|a| std::net::TcpStream::connect_timeout(&a, std::time::Duration::from_millis(1500)).is_ok()).unwrap_or(false);
            LAST.lock().1 = Some(ok);
        });
    }
    g.1
}

// ---- adding devices: what is around (read only: Jarvis does not pair or control them) ----
// «arp -a»: the lines «  192.168.1.5   aa-bb-cc-dd-ee-ff   dynamic» -> (address, hardware address); broadcast and multicast entries are left out
pub fn parse_arp(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 || !parts[2].eq_ignore_ascii_case("dynamic") && !parts[2].eq_ignore_ascii_case("динамический") {
            continue;
        }
        let ip = parts[0];
        let mac = parts[1].to_lowercase();
        let octets: Vec<u32> = ip.split('.').filter_map(|o| o.parse().ok()).collect();
        let ip_ok = octets.len() == 4 && octets.iter().all(|o| *o <= 255) && octets[3] != 255 && !(224..=239).contains(&octets[0]);
        let mac_ok = mac.len() == 17 && mac.split('-').count() == 6 && mac.split('-').all(|b| b.len() == 2 && b.chars().all(|c| c.is_ascii_hexdigit())) && mac != "ff-ff-ff-ff-ff-ff";
        if ip_ok && mac_ok {
            out.push((ip.to_string(), mac));
        }
    }
    out
}

// the devices of the local network (the table of addresses of Windows: those that talked to this computer lately)
pub fn lan_devices() -> Result<Vec<(String, String)>, String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let o = std::process::Command::new("arp").arg("-a").creation_flags(0x0800_0000).stdin(std::process::Stdio::null()).output().map_err(|e| e.to_string())?;
        Ok(parse_arp(&String::from_utf8_lossy(&o.stdout)))
    }
    #[cfg(not(target_os = "windows"))]
    Err("только в Windows".into())
}

// the Bluetooth devices Windows knows: «name|status» lines
pub fn parse_bluetooth(text: &str) -> Vec<(String, String)> {
    let mut seen = std::collections::HashSet::new();
    text.lines()
        .filter_map(|l| l.trim().split_once('|'))
        .map(|(n, st)| (n.trim().to_string(), st.trim().to_string()))
        // the adapters and the services of Windows are not devices of the user
        .filter(|(n, _)| !n.is_empty() && n.len() < 80 && !n.to_lowercase().contains("enumerator") && !n.to_lowercase().contains("transport") && !n.to_lowercase().contains("adapter") && !n.to_lowercase().contains("radio") && !n.to_lowercase().contains("generic"))
        .filter(|(n, _)| seen.insert(n.to_lowercase()))
        .collect()
}

pub fn bluetooth_devices() -> Result<Vec<(String, String)>, String> {
    #[cfg(target_os = "windows")]
    {
        use std::io::Read;
        use std::os::windows::process::CommandExt;
        let mut child = crate::psrun::command(crate::psrun::Script::Bluetooth, &[], crate::psrun::Way::default())?
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .creation_flags(0x0800_0000)
            .spawn()
            .map_err(|e| e.to_string())?;
        let mut out = child.stdout.take().ok_or("нет вывода")?;
        let reader = std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = out.read_to_end(&mut b);
            b
        });
        let t0 = std::time::Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if t0.elapsed() > std::time::Duration::from_secs(20) => {
                    let _ = child.kill();
                    return Err("Windows не ответила за 20 секунд".into());
                }
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
                Err(e) => return Err(e.to_string()),
            }
        }
        let bytes = reader.join().unwrap_or_default();
        Ok(parse_bluetooth(&String::from_utf8_lossy(&bytes)))
    }
    #[cfg(not(target_os = "windows"))]
    Err("только в Windows".into())
}

pub fn json() -> Value {
    json!({ "devices": to_json(&build(&gather())) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Inputs {
        Inputs { now: 10_000, pc_name: "PC-1".into(), user: "anna".into(), mic_selected: -1, ..Default::default() }
    }
    fn by_id<'a>(l: &'a [Device], id: &str) -> Option<&'a Device> {
        l.iter().find(|d| d.id == id)
    }
    fn state(d: &Device, label: &str) -> Cap {
        d.caps.iter().find(|c| c.label == label).unwrap_or_else(|| panic!("no capability {}", label)).state
    }

    #[test]
    fn a_desktop_with_nothing_else_lists_only_itself_and_invents_no_battery() {
        let l = build(&base());
        assert_eq!(l.len(), 1, "no phone, no audio device, no second computer is made up");
        let pc = &l[0];
        assert_eq!((pc.kind, pc.status, pc.battery), (Kind::Pc, Status::Online, None));
        assert_eq!(state(pc, "Заряд батареи"), Cap::NoData);
        assert_eq!(state(pc, "Микрофон"), Cap::Unavailable, "no microphone is shown as unavailable, never as working");
        assert_eq!(state(pc, "Удалённый доступ к камере, микрофону и экрану"), Cap::NotSupported);
    }

    #[test]
    fn a_computer_with_a_battery_is_a_laptop_with_the_real_charge() {
        let l = build(&Inputs { battery: Some((64, true)), mics: vec!["Mic".into()], ..base() });
        let pc = &l[0];
        assert_eq!((pc.kind, pc.battery, pc.charging), (Kind::Laptop, Some(64), Some(true)));
        assert_eq!(state(pc, "Микрофон"), Cap::Available);
        assert!(pc.caps.iter().all(|c| c.label != "Заряд батареи"), "the charge is shown as a number, not as a capability without data");
    }

    #[test]
    fn the_screen_picture_needs_the_permission_of_the_settings() {
        assert_eq!(state(&build(&base())[0], "Снимок экрана для ИИ"), Cap::NeedsPermission);
        assert_eq!(state(&build(&Inputs { screen_allowed: true, ..base() })[0], "Снимок экрана для ИИ"), Cap::Available);
    }

    #[test]
    fn a_phone_that_never_came_is_not_listed_and_paired_is_not_online() {
        assert!(by_id(&build(&Inputs { remote_on: true, ..base() }), "phone").is_none(), "an empty slot is not a device");
        // seen long ago, the remote is on: paired, offline
        let l = build(&Inputs { remote_on: true, phone_last_seen: Some(5_000), ..base() });
        let p = by_id(&l, "phone").unwrap();
        assert_eq!((p.paired, p.status, p.kind), (true, Status::Offline, Kind::Phone));
        assert_eq!(state(p, "Найти телефон (звуковой сигнал)"), Cap::Unavailable);
        assert_eq!(p.battery, None, "an old charge of an offline phone is not shown as current");
        // seen a few seconds ago but not polling now: reconnecting
        let l = build(&Inputs { remote_on: true, phone_last_seen: Some(9_985), ..base() });
        assert_eq!(by_id(&l, "phone").unwrap().status, Status::Connecting);
        // the remote switched off: the phone cannot be reached whatever its last word was
        let l = build(&Inputs { remote_on: false, phone_last_seen: Some(9_999), phone_online: true, ..base() });
        assert_eq!(by_id(&l, "phone").unwrap().status, Status::Offline);
    }

    #[test]
    fn an_online_phone_shows_only_what_it_really_reports() {
        let on = Inputs { remote_on: true, phone_last_seen: Some(9_999), phone_online: true, ..base() };
        let p = &build(&on)[1];
        assert_eq!((p.status, p.battery), (Status::Online, None));
        assert_eq!(state(p, "Заряд батареи"), Cap::NoData, "the browser did not send the charge");
        assert_eq!(state(p, "Найти телефон (звуковой сигнал)"), Cap::Available);
        assert_eq!(state(p, "Камера, микрофон и экран телефона"), Cap::NotSupported, "must not pretend to work");
        // the place is offered, but it needs the permission of the phone's owner on the page, and it is not shown until it comes
        assert_eq!(state(p, "Местоположение телефона"), Cap::NeedsPermission);
        assert!(p.location.is_none());
        let with_place = &build(&Inputs { phone_location: Some((41.3, 69.2, 20.0, 9_999)), phone_net: "wifi".into(), ..on.clone() })[1];
        assert_eq!(state(with_place, "Местоположение телефона"), Cap::Available);
        assert_eq!((with_place.location.map(|l| (l.0, l.1)), with_place.network.as_str()), (Some((41.3, 69.2)), "wifi"));
        let p2 = &build(&Inputs { phone_battery: Some(80), phone_charging: Some(false), ..on })[1];
        assert_eq!((p2.battery, p2.charging), (Some(80), Some(false)));
    }

    #[test]
    fn the_microphones_are_listed_with_the_chosen_one_and_no_remote_control_is_promised() {
        let l = build(&Inputs { mics: vec!["USB Headset".into(), "Built-in".into()], mic_selected: 1, ..base() });
        let a: Vec<&Device> = l.iter().filter(|d| d.kind == Kind::Audio).collect();
        assert_eq!(a.len(), 2);
        assert!(a[1].detail.contains("Выбран") && !a[0].detail.contains("Выбран"));
        assert!(a.iter().all(|d| state(d, "Удалённое управление") == Cap::NotSupported && d.battery.is_none()));
    }

    #[test]
    fn the_json_has_no_secrets_and_the_labels_are_russian() {
        let l = build(&Inputs { remote_on: true, phone_last_seen: Some(9_999), phone_online: true, mics: vec!["Mic".into()], ..base() });
        let text = to_json(&l).to_string();
        for secret in ["token", "secret", "key", "http://", "https://", "?t="] {
            assert!(!text.to_lowercase().contains(secret), "{} appears in the list of devices", secret);
        }
        let v = to_json(&l);
        assert_eq!(v[0]["status_label"], "в сети");
        assert!(v.as_array().unwrap().iter().all(|d| d["caps"].as_array().map(|c| !c.is_empty()).unwrap_or(false)));
    }

    // the model has room for the kinds that join later, with their own names
    #[test]
    fn the_kinds_the_architecture_allows_have_names() {
        for k in [Kind::Pc, Kind::Laptop, Kind::Phone, Kind::Tablet, Kind::Audio, Kind::Other] {
            assert!(!k.key().is_empty() && !k.label().is_empty());
        }
    }

    #[test]
    fn the_network_and_bluetooth_lists_keep_only_real_devices() {
        let arp = "Интерфейс: 192.168.1.4 --- 0x5\n  адрес в Интернете      Физический адрес      Тип\n  192.168.1.1           AA-BB-CC-DD-EE-01     динамический\n  192.168.1.9           aa-bb-cc-dd-ee-09     dynamic\n  192.168.1.255         ff-ff-ff-ff-ff-ff     static\n  224.0.0.22            01-00-5e-00-00-16     static\n  300.1.1.1             aa-bb-cc-dd-ee-10     dynamic\n  10.0.0.2              zz-bb-cc-dd-ee-11     dynamic\n";
        assert_eq!(parse_arp(arp), vec![("192.168.1.1".to_string(), "aa-bb-cc-dd-ee-01".to_string()), ("192.168.1.9".to_string(), "aa-bb-cc-dd-ee-09".to_string())]);
        let bt = "JBL Flip 5|OK\nMicrosoft Bluetooth Enumerator|OK\nIntel Wireless Bluetooth Adapter|OK\nJBL Flip 5|OK\nMi Band 6|Unknown\n\nrubbish line\n";
        assert_eq!(parse_bluetooth(bt), vec![("JBL Flip 5".to_string(), "OK".to_string()), ("Mi Band 6".to_string(), "Unknown".to_string())]);
    }
}
