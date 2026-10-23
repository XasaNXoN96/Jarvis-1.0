//! The numbers for the small line over the game: the load of the processor, the memory, the video card (an NVIDIA card through its own
//! library nvml.dll from the driver, nothing else is loaded or started) and the rate of the aim assist. Windows only; elsewhere zeros.
//!
//! The FRAME RATE OF THE GAME itself is not here and cannot be: it can only be read from inside the game (a hook) or from the tracing of
//! Windows that needs an administrator, and Jarvis does neither. The counters of Steam, of NVIDIA (Alt+R) or of the game show it.

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Sample {
    pub cpu_pct: u32,
    pub ram_used_gb: f32,
    pub ram_total_gb: f32,
    // None: no NVIDIA card or no library
    pub gpu_pct: Option<u32>,
    pub gpu_mem_used_gb: Option<f32>,
    pub gpu_mem_total_gb: Option<f32>,
    pub gpu_temp_c: Option<u32>,
    pub gpu_clock_mhz: Option<u32>,
    // the system drive
    pub disk_used_gb: Option<f32>,
    pub disk_total_gb: Option<f32>,
    // the current mode of the main display (Windows reports it; a display with a variable rate shows its mode, not the live rate)
    pub refresh_hz: Option<u32>,
}

// two readings closer than this are one: the numbers are shared by the window, the phone and the game line, and the load of the
// processor between two readings a few milliseconds apart is noise
pub const MIN_GAP_MS: u64 = 800;

pub fn reuse_cached(age_ms: u64) -> bool {
    age_ms < MIN_GAP_MS
}

// the frequency that Windows reports for the current mode: 0 and 1 mean «the default of the hardware», which is not a number
pub fn real_hz(v: u32) -> Option<u32> {
    if v > 1 && v <= 1000 { Some(v) } else { None }
}

// the load of the processor between two readings of (idle, kernel + user) times
pub fn cpu_percent(prev: (u64, u64), now: (u64, u64)) -> u32 {
    let (di, dt) = (now.0.saturating_sub(prev.0), now.1.saturating_sub(prev.1));
    if dt == 0 {
        return 0;
    }
    (((dt.saturating_sub(di)) as f64 / dt as f64) * 100.0).round().clamp(0.0, 100.0) as u32
}

fn gb(v: f32) -> String {
    // 12,4 (a comma, as the page writes); whole numbers from ten up
    if v >= 10.0 { format!("{:.0}", v) } else { format!("{:.1}", v).replace('.', ",") }
}

// «ЦП 23% · ОЗУ 12/32 ГБ · ВК 41% 3,2/8 ГБ · прицел 6 к/с»: short, to stand in one small line
pub fn line(s: &Sample, aim: Option<(u32, u64)>) -> String {
    line_with_fps(s, aim, None, None)
}

// the same with the frame rate of the game (when it is read) or the note why it is not
pub fn line_with_fps(s: &Sample, aim: Option<(u32, u64)>, game_fps: Option<u32>, note: Option<&str>) -> String {
    let mut t = String::new();
    if let Some(f) = game_fps {
        t.push_str(&format!("FPS {} · ", f));
    } else if let Some(n) = note {
        t.push_str(&format!("{} · ", n));
    }
    t.push_str(&format!("ЦП {}% · ОЗУ {}/{} ГБ", s.cpu_pct, gb(s.ram_used_gb), gb(s.ram_total_gb)));
    if let Some(g) = s.gpu_pct {
        t.push_str(&format!(" · ВК {}%", g));
        if let (Some(u), Some(tot)) = (s.gpu_mem_used_gb, s.gpu_mem_total_gb) {
            t.push_str(&format!(" {}/{} ГБ", gb(u), gb(tot)));
        }
    }
    if let Some((fps, ms)) = aim {
        t.push_str(&format!(" · прицел {} к/с ({} мс)", fps, ms));
    }
    t
}


// The video card of any maker (AMD, Intel, NVIDIA without its library): the counters of Windows «GPU Engine» and «GPU Adapter Memory».
// Each engine of each process is one instance; the load of the card is the sum of its 3D engines (what Task Manager shows as «3D»).
// The names look like «pid_1234_luid_0x0000_0x0000_phys_0_eng_0_engtype_3D».
pub fn gpu_load_3d(instances: &[(String, f64)]) -> Option<u32> {
    let v: Vec<f64> = instances.iter().filter(|(n, v)| n.to_lowercase().contains("engtype_3d") && v.is_finite() && *v >= 0.0).map(|(_, v)| *v).collect();
    if v.is_empty() {
        return None;
    }
    Some(v.iter().sum::<f64>().round().clamp(0.0, 100.0) as u32)
}

// the dedicated video memory in use, summed over the adapters (bytes -> GB); None when Windows gave nothing
pub fn gpu_mem_used(instances: &[(String, f64)]) -> Option<f32> {
    let v: Vec<f64> = instances.iter().filter(|(_, v)| v.is_finite() && *v >= 0.0).map(|(_, v)| *v).collect();
    if v.is_empty() {
        return None;
    }
    Some((v.iter().sum::<f64>() / 1024.0 / 1024.0 / 1024.0) as f32)
}

#[cfg(target_os = "windows")]
mod win {
    use super::*;
    use once_cell::sync::Lazy;
    use parking_lot::Mutex;
    use winapi::shared::minwindef::FILETIME;
    use winapi::um::libloaderapi::{GetProcAddress, LoadLibraryW};
    use winapi::um::processthreadsapi::GetSystemTimes;
    use winapi::um::sysinfoapi::{GetSystemDirectoryW, GlobalMemoryStatusEx, MEMORYSTATUSEX};

    static PREV: Lazy<Mutex<(u64, u64)>> = Lazy::new(|| Mutex::new((0, 0)));

    #[repr(C)]
    struct NvUtil {
        gpu: u32,
        memory: u32,
    }
    #[repr(C)]
    struct NvMem {
        total: u64,
        free: u64,
        used: u64,
    }
    type InitFn = unsafe extern "C" fn() -> i32;
    type HandleFn = unsafe extern "C" fn(u32, *mut *mut std::ffi::c_void) -> i32;
    type UtilFn = unsafe extern "C" fn(*mut std::ffi::c_void, *mut NvUtil) -> i32;
    type MemFn = unsafe extern "C" fn(*mut std::ffi::c_void, *mut NvMem) -> i32;
    type TempFn = unsafe extern "C" fn(*mut std::ffi::c_void, u32, *mut u32) -> i32;
    type ClockFn = unsafe extern "C" fn(*mut std::ffi::c_void, u32, *mut u32) -> i32;

    struct Nvml {
        handle: usize,
        util: UtilFn,
        mem: MemFn,
        temp: Option<TempFn>,
        clock: Option<ClockFn>,
    }
    // the library of the driver is loaded once, from the system folder only (never from the folder of the program or the current one)
    static NVML: Lazy<Option<Nvml>> = Lazy::new(|| unsafe {
        let mut dir = [0u16; 260];
        let n = GetSystemDirectoryW(dir.as_mut_ptr(), 260) as usize;
        if n == 0 || n > 240 {
            return None;
        }
        let mut path: Vec<u16> = dir[..n].to_vec();
        path.extend("\\nvml.dll".encode_utf16());
        path.push(0);
        let lib = LoadLibraryW(path.as_ptr());
        if lib.is_null() {
            return None;
        }
        let sym = |name: &[u8]| GetProcAddress(lib, name.as_ptr() as *const i8);
        let (init, handle, util, mem) = (sym(b"nvmlInit_v2\0"), sym(b"nvmlDeviceGetHandleByIndex_v2\0"), sym(b"nvmlDeviceGetUtilizationRates\0"), sym(b"nvmlDeviceGetMemoryInfo\0"));
        if init.is_null() || handle.is_null() || util.is_null() || mem.is_null() {
            return None;
        }
        let (init, handle, util, mem): (InitFn, HandleFn, UtilFn, MemFn) = (std::mem::transmute(init), std::mem::transmute(handle), std::mem::transmute(util), std::mem::transmute(mem));
        if init() != 0 {
            return None;
        }
        let mut h: *mut std::ffi::c_void = std::ptr::null_mut();
        if handle(0, &mut h) != 0 || h.is_null() {
            return None;
        }
        // (the temperature and the clock are optional: an old driver without them still gives the load)
        let (t, c) = (sym(b"nvmlDeviceGetTemperature\0"), sym(b"nvmlDeviceGetClockInfo\0"));
        let temp: Option<TempFn> = if t.is_null() { None } else { Some(std::mem::transmute(t)) };
        let clock: Option<ClockFn> = if c.is_null() { None } else { Some(std::mem::transmute(c)) };
        Some(Nvml { handle: h as usize, util, mem, temp, clock })
    });


    // the counters are opened once; a rate counter needs two readings, so the first one gives nothing (None) and the next ones do
    struct Pdh {
        query: winapi::um::pdh::PDH_HQUERY,
        engine: winapi::um::pdh::PDH_HCOUNTER,
        memory: winapi::um::pdh::PDH_HCOUNTER,
    }
    unsafe impl Send for Pdh {}
    static PDH: Lazy<Mutex<Option<Pdh>>> = Lazy::new(|| unsafe {
        use winapi::um::pdh::*;
        let mut q: PDH_HQUERY = std::ptr::null_mut();
        if PdhOpenQueryW(std::ptr::null(), 0, &mut q) != 0 {
            return Mutex::new(None);
        }
        let wide = |t: &str| -> Vec<u16> { t.encode_utf16().chain(std::iter::once(0)).collect() };
        let (mut e, mut m): (PDH_HCOUNTER, PDH_HCOUNTER) = (std::ptr::null_mut(), std::ptr::null_mut());
        // (a computer without these counters — an old Windows — simply has none: the card is then «Недоступно»)
        if PdhAddEnglishCounterW(q, wide("\\GPU Engine(*)\\Utilization Percentage").as_ptr(), 0, &mut e) != 0 {
            e = std::ptr::null_mut();
        }
        if PdhAddEnglishCounterW(q, wide("\\GPU Adapter Memory(*)\\Dedicated Usage").as_ptr(), 0, &mut m) != 0 {
            m = std::ptr::null_mut();
        }
        if e.is_null() && m.is_null() {
            PdhCloseQuery(q);
            return Mutex::new(None);
        }
        Mutex::new(Some(Pdh { query: q, engine: e, memory: m }))
    });

    // (the status «the buffer is too small, here is the size»; winapi does not declare it)
    const PDH_MORE_DATA: i32 = 0x800007D2u32 as i32;

    unsafe fn instances(c: winapi::um::pdh::PDH_HCOUNTER) -> Vec<(String, f64)> {
        use winapi::um::pdh::*;
        if c.is_null() {
            return Vec::new();
        }
        let fmt = PDH_FMT_DOUBLE | PDH_FMT_NOCAP100;
        let (mut size, mut count) = (0u32, 0u32);
        if PdhGetFormattedCounterArrayW(c, fmt, &mut size, &mut count, std::ptr::null_mut()) != PDH_MORE_DATA || size == 0 || size > 8_000_000 {
            return Vec::new();
        }
        // (u64 so that the buffer is aligned for the items)
        let mut buf = vec![0u64; (size as usize + 7) / 8];
        let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
        if PdhGetFormattedCounterArrayW(c, fmt, &mut size, &mut count, items) != 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        for i in 0..count as usize {
            let it = &*items.add(i);
            if it.FmtValue.CStatus != 0 || it.szName.is_null() {
                continue;
            }
            let mut n = 0usize;
            while n < 400 && *it.szName.add(n) != 0 {
                n += 1;
            }
            let name = String::from_utf16_lossy(std::slice::from_raw_parts(it.szName, n));
            out.push((name, *it.FmtValue.u.doubleValue()));
        }
        out
    }

    // (load, memory in use) of the video card from the counters of Windows
    fn pdh_gpu() -> (Option<u32>, Option<f32>) {
        let g = PDH.lock();
        let Some(p) = g.as_ref() else { return (None, None) };
        unsafe {
            if winapi::um::pdh::PdhCollectQueryData(p.query) != 0 {
                return (None, None);
            }
            (gpu_load_3d(&instances(p.engine)), gpu_mem_used(&instances(p.memory)))
        }
    }

    pub fn sample() -> Sample {
        static CACHE: Lazy<Mutex<Option<(std::time::Instant, Sample)>>> = Lazy::new(|| Mutex::new(None));
        // (held during the reading: two callers at once get one reading and the load of the processor stays meaningful)
        let mut c = CACHE.lock();
        if let Some((at, s)) = c.as_ref() {
            if reuse_cached(at.elapsed().as_millis() as u64) {
                return s.clone();
            }
        }
        let s = read();
        *c = Some((std::time::Instant::now(), s.clone()));
        s
    }

    fn read() -> Sample {
        let mut s = Sample::default();
        unsafe {
            {
                use winapi::um::wingdi::DEVMODEW;
                use winapi::um::winuser::{EnumDisplaySettingsW, ENUM_CURRENT_SETTINGS};
                let mut dm: DEVMODEW = std::mem::zeroed();
                dm.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
                if EnumDisplaySettingsW(std::ptr::null(), ENUM_CURRENT_SETTINGS, &mut dm) != 0 {
                    s.refresh_hz = real_hz(dm.dmDisplayFrequency);
                }
            }
            let mut m: MEMORYSTATUSEX = std::mem::zeroed();
            m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
            GlobalMemoryStatusEx(&mut m);
            let g = |b: u64| b as f32 / 1024.0 / 1024.0 / 1024.0;
            s.ram_total_gb = g(m.ullTotalPhys);
            s.ram_used_gb = g(m.ullTotalPhys - m.ullAvailPhys);
            let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
            let (mut idle, mut kernel, mut user): (FILETIME, FILETIME, FILETIME) = std::mem::zeroed();
            if GetSystemTimes(&mut idle, &mut kernel, &mut user) != 0 {
                let now = (ft(idle), ft(kernel) + ft(user));
                let prev = std::mem::replace(&mut *PREV.lock(), now);
                if prev.1 > 0 {
                    s.cpu_pct = cpu_percent(prev, now);
                }
            }
            // the drive Windows is installed on
            {
                use winapi::um::fileapi::GetDiskFreeSpaceExW;
                use winapi::shared::ntdef::ULARGE_INTEGER;
                let root: Vec<u16> = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()).encode_utf16().chain("\\".encode_utf16()).chain(std::iter::once(0)).collect();
                let (mut free, mut total, mut tf): (ULARGE_INTEGER, ULARGE_INTEGER, ULARGE_INTEGER) = std::mem::zeroed();
                if GetDiskFreeSpaceExW(root.as_ptr(), &mut free, &mut total, &mut tf) != 0 {
                    let (t, f) = (*total.QuadPart(), *free.QuadPart());
                    if t > 0 {
                        s.disk_total_gb = Some(g(t));
                        s.disk_used_gb = Some(g(t.saturating_sub(f)));
                    }
                }
            }
            if let Some(n) = NVML.as_ref() {
                let h = n.handle as *mut std::ffi::c_void;
                let mut u = NvUtil { gpu: 0, memory: 0 };
                if (n.util)(h, &mut u) == 0 {
                    s.gpu_pct = Some(u.gpu.min(100));
                }
                let mut mm = NvMem { total: 0, free: 0, used: 0 };
                if (n.mem)(h, &mut mm) == 0 && mm.total > 0 {
                    s.gpu_mem_used_gb = Some(g(mm.used));
                    s.gpu_mem_total_gb = Some(g(mm.total));
                }
                if let Some(f) = n.temp {
                    let mut t = 0u32;
                    if f(h, 0, &mut t) == 0 && t > 0 && t < 150 {
                        s.gpu_temp_c = Some(t);
                    }
                }
                if let Some(f) = n.clock {
                    let mut c = 0u32;
                    if f(h, 0, &mut c) == 0 && c > 0 {
                        s.gpu_clock_mhz = Some(c);
                    }
                }
            } else {
                // not an NVIDIA card (or no library): the counters of Windows give the load and the memory in use (no total, no temperature)
                let (load, used) = pdh_gpu();
                s.gpu_pct = load;
                s.gpu_mem_used_gb = used;
            }
        }
        s
    }
}

#[cfg(target_os = "windows")]
pub fn sample() -> Sample {
    win::sample()
}

#[cfg(not(target_os = "windows"))]
pub fn sample() -> Sample {
    Sample::default()
}

// The block of the home page («Мой ПК»): every value is real or marked «Недоступно» with the reason; nothing is invented.
// The temperature of the processor cannot be read without a driver of the board maker, so it is never shown.
pub fn monitor_json(s: &Sample, available: bool) -> serde_json::Value {
    use serde_json::json;
    let gbs = |v: f32| format!("{} ГБ", gb(v));
    let na = |id: &str, label: &str, why: &str| json!({ "id": id, "label": label, "value": "Недоступно", "pct": null, "available": false, "why": why });
    let ok = |id: &str, label: &str, value: String, pct: Option<u32>| json!({ "id": id, "label": label, "value": value, "pct": pct, "available": true });
    if !available {
        return json!({ "items": [na("cpu", "Процессор", "только в Windows"), na("ram", "Память", "только в Windows")] });
    }
    let mut items = vec![ok("cpu", "Процессор", format!("{}%", s.cpu_pct), Some(s.cpu_pct))];
    items.push(if s.ram_total_gb > 0.0 { ok("ram", "Память", format!("{} / {}", gbs(s.ram_used_gb), gbs(s.ram_total_gb)), Some((s.ram_used_gb / s.ram_total_gb * 100.0).round().clamp(0.0, 100.0) as u32)) } else { na("ram", "Память", "Windows не ответил") });
    items.push(match (s.disk_used_gb, s.disk_total_gb) {
        (Some(u), Some(t)) if t > 0.0 => ok("disk", "Диск системы", format!("{} / {}", gbs(u), gbs(t)), Some((u / t * 100.0).round().clamp(0.0, 100.0) as u32)),
        _ => na("disk", "Диск системы", "Windows не ответил"),
    });
    let nv = "Windows не отдаёт счётчики видеокарты (нужен Windows 10 версии 1709 или новее и драйвер видеокарты)";
    items.push(match s.gpu_pct { Some(g) => ok("gpu", "Видеокарта", format!("{}%", g), Some(g)), None => na("gpu", "Видеокарта", nv) });
    items.push(match (s.gpu_mem_used_gb, s.gpu_mem_total_gb) {
        (Some(u), Some(t)) if t > 0.0 => ok("vram", "Видеопамять", format!("{} / {}", gbs(u), gbs(t)), Some((u / t * 100.0).round().clamp(0.0, 100.0) as u32)),
        // a card that is not NVIDIA: Windows tells what is in use, not the total
        (Some(u), _) => ok("vram", "Видеопамять", format!("занято {}", gbs(u)), None),
        _ => na("vram", "Видеопамять", nv),
    });
    items.push(match s.gpu_temp_c { Some(t) => ok("gpu_temp", "Температура видеокарты", format!("{} °C", t), Some(t.min(100))), None => na("gpu_temp", "Температура видеокарты", nv) });
    items.push(match s.gpu_clock_mhz { Some(c) => ok("gpu_clock", "Частота видеокарты", format!("{} МГц", c), None), None => na("gpu_clock", "Частота видеокарты", nv) });
    items.push(match s.refresh_hz { Some(h) => ok("refresh", "Частота экрана", format!("{} Гц", h), None), None => na("refresh", "Частота экрана", "Windows не сообщил частоту режима экрана") });
    json!({ "items": items })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readings_are_shared_and_the_display_rate_is_never_invented() {
        assert!(reuse_cached(0) && reuse_cached(MIN_GAP_MS - 1));
        assert!(!reuse_cached(MIN_GAP_MS), "after the gap a new reading is made");
        assert_eq!((real_hz(0), real_hz(1), real_hz(60), real_hz(144), real_hz(99_999)), (None, None, Some(60), Some(144), None));
        let na = monitor_json(&Sample { cpu_pct: 1, ram_total_gb: 8.0, ..Default::default() }, true);
        let r = na["items"].as_array().unwrap().iter().find(|i| i["id"] == "refresh").unwrap();
        assert_eq!((r["value"].as_str(), r["available"].as_bool()), (Some("Недоступно"), Some(false)));
        let yes = monitor_json(&Sample { cpu_pct: 1, ram_total_gb: 8.0, refresh_hz: Some(144), ..Default::default() }, true);
        let r = yes["items"].as_array().unwrap().iter().find(|i| i["id"] == "refresh").unwrap();
        assert_eq!(r["value"], "144 Гц");
    }

    #[test]
    fn the_load_of_any_video_card_is_the_sum_of_its_3d_engines() {
        let i = |n: &str, v: f64| (n.to_string(), v);
        let l = vec![i("pid_1_luid_0x0_0x1_phys_0_eng_0_engtype_3D", 30.4), i("pid_2_luid_0x0_0x1_phys_0_eng_0_engtype_3D", 12.2), i("pid_2_luid_0x0_0x1_phys_0_eng_3_engtype_VideoDecode", 90.0), i("pid_3_eng_1_engtype_Copy", 50.0)];
        assert_eq!(gpu_load_3d(&l), Some(43), "only the 3D engines count");
        assert_eq!(gpu_load_3d(&[i("a_engtype_3D", 80.0), i("b_engtype_3D", 70.0)]), Some(100), "never above 100");
        assert_eq!(gpu_load_3d(&[i("a_engtype_Copy", 5.0)]), None, "no 3D engine: no number is invented");
        assert_eq!(gpu_load_3d(&[]), None);
        assert_eq!(gpu_load_3d(&[i("a_engtype_3D", f64::NAN), i("b_engtype_3D", -3.0)]), None, "garbage is ignored");
        assert_eq!(gpu_mem_used(&[]), None);
        let gb = gpu_mem_used(&[i("luid_a", 1_073_741_824.0), i("luid_b", 536_870_912.0)]).unwrap();
        assert!((gb - 1.5).abs() < 0.001, "{}", gb);
    }

    #[test]
    fn a_card_without_a_known_total_shows_what_is_in_use() {
        let s = Sample { cpu_pct: 1, ram_total_gb: 8.0, gpu_pct: Some(10), gpu_mem_used_gb: Some(2.0), ..Default::default() };
        let m = monitor_json(&s, true);
        let v = m["items"].as_array().unwrap().iter().find(|i| i["id"] == "vram").unwrap();
        assert_eq!((v["available"].as_bool(), v["value"].as_str().map(|t| t.starts_with("занято"))), (Some(true), Some(true)));
        assert!(v["pct"].is_null(), "no percent without a total");
    }

    #[test]
    fn the_load_of_the_processor_is_the_busy_part_of_the_time_between_two_readings() {
        assert_eq!(cpu_percent((100, 1000), (150, 1100)), 50);
        assert_eq!(cpu_percent((100, 1000), (200, 1100)), 0, "all of it was idle");
        assert_eq!(cpu_percent((100, 1000), (100, 1100)), 100);
        assert_eq!(cpu_percent((5, 5), (5, 5)), 0, "no time has passed");
        assert_eq!(cpu_percent((500, 500), (0, 0)), 0, "a counter that went back is not a load");
    }

    #[test]
    fn the_line_is_short_and_has_only_what_is_known() {
        let s = Sample { cpu_pct: 23, ram_used_gb: 12.4, ram_total_gb: 31.9, gpu_pct: Some(41), gpu_mem_used_gb: Some(3.2), gpu_mem_total_gb: Some(8.0), ..Default::default() };
        assert_eq!(line(&s, Some((6, 160))), "ЦП 23% · ОЗУ 12/32 ГБ · ВК 41% 3,2/8,0 ГБ · прицел 6 к/с (160 мс)");
        // no video card of NVIDIA, no aim assist: the line is only the processor and the memory
        let t = Sample { cpu_pct: 5, ram_used_gb: 8.5, ram_total_gb: 16.0, ..Default::default() };
        assert_eq!(line(&t, None), "ЦП 5% · ОЗУ 8,5/16 ГБ");
        assert_eq!(line_with_fps(&t, None, Some(144), None), "FPS 144 · ЦП 5% · ОЗУ 8,5/16 ГБ");
        assert!(line_with_fps(&t, None, None, Some("FPS игры: нужен запуск от администратора")).starts_with("FPS игры: нужен"));
        assert!(line(&s, None).chars().count() < 60, "one small line");
    }

    #[test]
    fn the_monitor_block_marks_what_is_unknown_and_invents_nothing() {
        let t = Sample { cpu_pct: 5, ram_used_gb: 8.0, ram_total_gb: 16.0, ..Default::default() };
        let j = monitor_json(&t, true);
        let items = j["items"].as_array().unwrap();
        let get = |id: &str| items.iter().find(|i| i["id"] == id).unwrap().clone();
        assert_eq!(get("cpu")["value"], "5%");
        assert_eq!(get("ram")["pct"], 50);
        assert_eq!(get("gpu")["value"], "Недоступно");
        assert_eq!(get("gpu")["available"], false);
        assert_eq!(get("disk")["value"], "Недоступно");
        let full = Sample { gpu_pct: Some(41), gpu_mem_used_gb: Some(3.0), gpu_mem_total_gb: Some(8.0), gpu_temp_c: Some(62), gpu_clock_mhz: Some(1800), disk_used_gb: Some(100.0), disk_total_gb: Some(500.0), ..t };
        let j = monitor_json(&full, true);
        let g = |id: &str| j["items"].as_array().unwrap().iter().find(|i| i["id"] == id).unwrap().clone();
        assert_eq!(g("gpu_temp")["value"], "62 °C");
        assert_eq!(g("vram")["pct"], 38);
        assert_eq!(g("disk")["pct"], 20);
        assert_eq!(g("gpu_clock")["value"], "1800 МГц");
        assert!(monitor_json(&t, false)["items"][0]["available"] == false);
    }

    #[test]
    fn a_sample_can_be_taken_on_any_system() {
        let s = sample();
        assert!(s.cpu_pct <= 100);
    }
}
