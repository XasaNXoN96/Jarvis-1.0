// "System Health" of the home page: only real readings. What cannot be read is reported as unavailable (null), never invented.
// CPU / memory / disk come from the same code the "Панель" page uses (uistate::stats); the temperature is read in a background
// thread every half minute (a PowerShell / WMI call is too slow to make on every poll); the network state is the
// system's own routing table (no packet is sent).

use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::{json, Value};

static TEMP: Lazy<Mutex<Option<f32>>> = Lazy::new(|| Mutex::new(None));
static SAMPLER: AtomicBool = AtomicBool::new(false);

// "OK" / "HIGH LOAD" / "LOW MEMORY" / "LOW DISK": a summary of the three readings that are real
pub fn pc_status(cpu: f32, ram_pct: f32, disk_pct: f32) -> &'static str {
    if disk_pct >= 95.0 {
        "LOW DISK"
    } else if ram_pct >= 92.0 {
        "LOW MEMORY"
    } else if cpu >= 90.0 {
        "HIGH LOAD"
    } else {
        "OK"
    }
}

// the temperature in degrees Celsius from the text a sensor returned (a number, Kelvin or tenths of Kelvin)
pub fn celsius_from(raw: &str) -> Option<f32> {
    let v: f32 = raw.trim().replace(',', ".").parse().ok()?;
    let c = if v > 2000.0 {
        v / 10.0 - 273.15 // tenths of Kelvin (MSAcpi_ThermalZoneTemperature)
    } else if v > 200.0 {
        v - 273.15 // Kelvin
    } else {
        v
    };
    if (1.0..=125.0).contains(&c) { Some((c * 10.0).round() / 10.0) } else { None }
}

#[cfg(target_os = "windows")]
fn read_temperature() -> Option<f32> {
    use std::os::windows::process::CommandExt;
    // two sources: the thermal zone counters (no admin needed on most PCs) and ACPI (often needs admin)
    let scripts = [
        "(Get-CimInstance Win32_PerfFormattedData_Counters_ThermalZoneInformation -ErrorAction Stop | Select-Object -First 1).Temperature",
        "(Get-CimInstance -Namespace root/wmi -ClassName MSAcpi_ThermalZoneTemperature -ErrorAction Stop | Select-Object -First 1).CurrentTemperature",
    ];
    for script in scripts {
        let out = std::process::Command::new("powershell").args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script]).creation_flags(0x0800_0000).output().ok()?;
        if out.status.success() {
            if let Some(c) = celsius_from(&String::from_utf8_lossy(&out.stdout)) {
                return Some(c);
            }
        }
    }
    None
}

#[cfg(not(target_os = "windows"))]
fn read_temperature() -> Option<f32> {
    let mut best: Option<f32> = None;
    for entry in std::fs::read_dir("/sys/class/thermal").ok()?.flatten() {
        let path = entry.path().join("temp");
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Ok(milli) = text.trim().parse::<f32>() {
                let c = milli / 1000.0;
                if (1.0..=125.0).contains(&c) {
                    best = Some(best.map_or(c, |b: f32| b.max(c)));
                }
            }
        }
    }
    best
}

fn start_sampler() {
    if SAMPLER.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| loop {
        *TEMP.lock() = read_temperature();
        std::thread::sleep(Duration::from_secs(30));
    });
}

// (online, local address): is there a route to the Internet. A UDP "connect" sends nothing, it only asks the system.
pub fn network() -> (bool, Option<String>) {
    match UdpSocket::bind("0.0.0.0:0").and_then(|s| s.connect("8.8.8.8:53").map(|_| s)) {
        Ok(s) => (true, s.local_addr().ok().map(|a| a.ip().to_string())),
        Err(_) => (false, None),
    }
}

pub fn json() -> Value {
    start_sampler();
    let st = crate::uistate::stats();
    let ram_pct = if st.mem_total_gb > 0.0 { st.mem_used_gb / st.mem_total_gb * 100.0 } else { 0.0 };
    let (disk_pct, disk_known) = if st.disk_total_gb > 0.0 { ((st.disk_total_gb - st.disk_free_gb) / st.disk_total_gb * 100.0, true) } else { (0.0, false) };
    let (online, ip) = network();
    let temp = *TEMP.lock();
    json!({
        "cpu": st.cpu.round(),
        "ram_pct": ram_pct.round(), "ram_used": (st.mem_used_gb * 10.0).round() / 10.0, "ram_total": (st.mem_total_gb * 10.0).round() / 10.0,
        "disk_pct": if disk_known { json!(disk_pct.round()) } else { Value::Null },
        "disk_free": (st.disk_free_gb * 10.0).round() / 10.0, "disk_total": (st.disk_total_gb * 10.0).round() / 10.0,
        "temperature": temp,
        "network": if online { "ONLINE" } else { "OFFLINE" }, "ip": ip,
        "pc_status": pc_status(st.cpu, ram_pct, if disk_known { disk_pct } else { 0.0 }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pc_status_follows_the_readings() {
        assert_eq!(pc_status(10.0, 40.0, 50.0), "OK");
        assert_eq!(pc_status(95.0, 40.0, 50.0), "HIGH LOAD");
        assert_eq!(pc_status(10.0, 95.0, 50.0), "LOW MEMORY");
        assert_eq!(pc_status(99.0, 99.0, 97.0), "LOW DISK");
    }

    #[test]
    fn temperature_units_are_recognized_and_nonsense_is_rejected() {
        assert_eq!(celsius_from("3182"), Some(45.1)); // tenths of Kelvin
        assert_eq!(celsius_from("318.15"), Some(45.0)); // Kelvin
        assert_eq!(celsius_from("52"), Some(52.0));
        assert_eq!(celsius_from(""), None);
        assert_eq!(celsius_from("abc"), None);
        assert_eq!(celsius_from("0"), None);
        assert_eq!(celsius_from("9999"), None);
    }

    #[test]
    fn health_json_has_only_real_fields() {
        let v = json();
        assert!(v["ram_total"].as_f64().unwrap() > 0.0, "memory is read from the system");
        assert!(["ONLINE", "OFFLINE"].contains(&v["network"].as_str().unwrap()));
        // an unknown temperature is null, never a made-up number
        assert!(v["temperature"].is_null() || v["temperature"].as_f64().unwrap() > 0.0);
    }
}
