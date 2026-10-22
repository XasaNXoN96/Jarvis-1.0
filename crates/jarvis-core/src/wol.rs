//! Wake-on-LAN: wakes ANOTHER computer of the home network by the «magic packet» (6 bytes of 0xFF and the MAC address 16 times, sent by
//! UDP broadcast). It works only if the target computer has Wake-on-LAN switched on (BIOS and network card) and is on the same network (usually by cable).
//! Jarvis can send the packet; it cannot know whether the computer woke up, and says so.

use crate::{store, textnorm};
use serde::{Deserialize, Serialize};

const FILE: &str = "wol_devices.json";
pub const MAX_DEVICES: usize = 30;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Target {
    pub id: String,
    pub name: String,
    // "AA:BB:CC:DD:EE:FF"
    pub mac: String,
}

// 6 bytes of a MAC from «aa:bb:cc:dd:ee:ff», «AA-BB-…» or «aabbccddeeff»
pub fn parse_mac(s: &str) -> Option<[u8; 6]> {
    let hex: String = s.trim().chars().filter(|c| *c != ':' && *c != '-' && *c != '.').collect();
    if hex.len() != 12 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 6];
    for i in 0..6 {
        out[i] = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
    }
    // all-zero and all-ff are not addresses of a card
    if out == [0; 6] || out == [0xff; 6] {
        return None;
    }
    Some(out)
}

pub fn show_mac(m: &[u8; 6]) -> String {
    m.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(":")
}

pub fn magic_packet(mac: &[u8; 6]) -> Vec<u8> {
    let mut p = vec![0xffu8; 6];
    for _ in 0..16 {
        p.extend_from_slice(mac);
    }
    p
}

pub fn list() -> Vec<Target> {
    store::read::<Vec<Target>>(FILE).0
}

pub fn add(name: &str, mac: &str) -> Result<Target, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 40 {
        return Err("Дайте компьютеру название (до 40 символов).".into());
    }
    let m = parse_mac(mac).ok_or("MAC-адрес в виде AA:BB:CC:DD:EE:FF (его видно в «ipconfig /all» на том компьютере).")?;
    let mut l = list();
    if l.len() >= MAX_DEVICES {
        return Err("Слишком много устройств.".into());
    }
    let mac = show_mac(&m);
    if l.iter().any(|t| t.mac == mac) {
        return Err("Этот адрес уже добавлен.".into());
    }
    let t = Target { id: store::new_id(), name: name.to_string(), mac };
    l.push(t.clone());
    store::write(FILE, &l)?;
    Ok(t)
}

pub fn remove(id: &str) -> bool {
    let mut l = list();
    let n = l.len();
    l.retain(|t| t.id != id);
    l.len() != n && store::write(FILE, &l).is_ok()
}

// sends the packet to the broadcast address; Ok = the packet left this computer (not that the other one woke up)
pub fn send(mac: &[u8; 6], dest: &str) -> Result<(), String> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("сеть недоступна: {}", e))?;
    sock.set_broadcast(true).map_err(|e| e.to_string())?;
    let packet = magic_packet(mac);
    sock.send_to(&packet, dest).map(|_| ()).map_err(|e| format!("пакет не отправился: {}", e))
}

pub fn wake(id: &str) -> Result<String, String> {
    crate::owner::guard()?;
    let t = list().into_iter().find(|t| t.id == id).ok_or("Такого устройства нет в списке.")?;
    let mac = parse_mac(&t.mac).ok_or("Сохранён неверный адрес.")?;
    send(&mac, "255.255.255.255:9")?;
    Ok(format!("Отправил сигнал пробуждения для «{}». Проснётся ли он, я не узнаю: нужно, чтобы в BIOS и в сетевой карте был включён Wake-on-LAN, а компьютер был в этой же сети.", t.name))
}

// «разбуди компьютер папы» / «wake up office pc»
pub fn parse(text: &str) -> Option<String> {
    let n = textnorm::normalize(text);
    for p in ["разбуди компьютер ", "разбуди ", "включи по сети ", "wake up ", "wake "] {
        if let Some(r) = n.strip_prefix(p) {
            return Some(r.to_string()).filter(|r| !r.is_empty());
        }
    }
    None
}

// a device by the spoken name; None when nothing fits (the phrase is then not for us)
pub fn find(name: &str) -> Option<Target> {
    let l = list();
    let names: Vec<&str> = l.iter().map(|t| t.name.as_str()).collect();
    match textnorm::best_match(name, &names) {
        textnorm::Match::Exact(i) | textnorm::Match::Fuzzy(i, _) => Some(l[i].clone()),
        _ => {
            // the spoken name inside the saved one («папы» in «Компьютер папы») or the saved one inside the spoken phrase
            let q = textnorm::normalize(name);
            let hits: Vec<&Target> = l.iter().filter(|t| { let n = textnorm::normalize(&t.name); !n.is_empty() && !q.is_empty() && (q.contains(&n) || (q.chars().count() >= 3 && n.split(' ').any(|w| w == q))) }).collect();
            if hits.len() == 1 { Some(hits[0].clone()) } else { None }
        }
    }
}

pub fn handle(text: &str) -> Option<String> {
    let name = parse(text)?;
    let t = find(&name)?;
    Some(wake(&t.id).unwrap_or_else(|e| e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_addresses_are_read_in_every_usual_spelling_and_bad_ones_refused() {
        let want = [0xAA, 0xBB, 0xCC, 0x01, 0x02, 0x03];
        for s in ["aa:bb:cc:01:02:03", "AA-BB-CC-01-02-03", "aabbcc010203", " aabb.cc01.0203 "] {
            assert_eq!(parse_mac(s), Some(want), "{}", s);
        }
        for s in ["", "aa:bb", "zz:bb:cc:01:02:03", "00:00:00:00:00:00", "ff:ff:ff:ff:ff:ff", "aa:bb:cc:01:02:03:04"] {
            assert_eq!(parse_mac(s), None, "{}", s);
        }
        assert_eq!(show_mac(&want), "AA:BB:CC:01:02:03");
    }

    #[test]
    fn the_magic_packet_is_exactly_102_bytes_in_the_known_shape() {
        let mac = [1, 2, 3, 4, 5, 6];
        let p = magic_packet(&mac);
        assert_eq!(p.len(), 102);
        assert_eq!(&p[..6], &[0xff; 6]);
        assert!(p[6..].chunks(6).all(|c| c == mac));
    }

    #[test]
    fn the_list_is_kept_and_checked() {
        store::with_temp(|| {
            assert!(add("", "aa:bb:cc:01:02:03").is_err());
            assert!(add("Офис", "xx").is_err());
            let t = add("Компьютер папы", "aa-bb-cc-01-02-03").unwrap();
            assert_eq!(t.mac, "AA:BB:CC:01:02:03");
            assert!(add("Другой", "AA:BB:CC:01:02:03").unwrap_err().contains("уже"));
            assert_eq!(find("папы").unwrap().id, t.id);
            assert_eq!(find("компьютер папы").unwrap().id, t.id);
            assert!(find("бабушки").is_none());
            assert!(remove(&t.id) && list().is_empty());
        });
    }

    #[test]
    fn the_packet_really_leaves_over_udp() {
        // a local receiver instead of the broadcast address
        let rx = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        rx.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
        let addr = rx.local_addr().unwrap().to_string();
        let mac = [0xde, 0xad, 0xbe, 0xef, 0x00, 0x01];
        send(&mac, &addr).unwrap();
        let mut buf = [0u8; 200];
        let n = rx.recv(&mut buf).unwrap();
        assert_eq!(&buf[..n], magic_packet(&mac).as_slice());
    }

    #[test]
    fn phrases_and_honesty() {
        assert_eq!(parse("Разбуди компьютер папы"), Some("папы".into()));
        assert_eq!(parse("открой браузер"), None);
        store::with_temp(|| {
            crate::owner::reset_for_tests();
            assert!(handle("разбуди кого-то").is_none(), "an unknown name is not ours");
            add("Офис", "aa:bb:cc:01:02:03").unwrap();
            // (the real broadcast may be impossible in a sandbox; the answer is either the honest «отправил» or the honest error)
            let r = handle("разбуди офис").unwrap();
            assert!(r.contains("Отправил сигнал") && r.contains("не узнаю") || r.contains("не отправился") || r.contains("сеть"), "{}", r);
        });
    }
}
