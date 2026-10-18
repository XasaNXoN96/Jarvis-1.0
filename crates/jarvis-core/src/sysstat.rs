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

    struct Nvml {
        handle: usize,
        util: UtilFn,
        mem: MemFn,
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
        Some(Nvml { handle: h as usize, util, mem })
    });

    pub fn sample() -> Sample {
        let mut s = Sample::default();
        unsafe {
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let s = Sample { cpu_pct: 23, ram_used_gb: 12.4, ram_total_gb: 31.9, gpu_pct: Some(41), gpu_mem_used_gb: Some(3.2), gpu_mem_total_gb: Some(8.0) };
        assert_eq!(line(&s, Some((6, 160))), "ЦП 23% · ОЗУ 12/32 ГБ · ВК 41% 3,2/8,0 ГБ · прицел 6 к/с (160 мс)");
        // no video card of NVIDIA, no aim assist: the line is only the processor and the memory
        let t = Sample { cpu_pct: 5, ram_used_gb: 8.5, ram_total_gb: 16.0, ..Default::default() };
        assert_eq!(line(&t, None), "ЦП 5% · ОЗУ 8,5/16 ГБ");
        assert_eq!(line_with_fps(&t, None, Some(144), None), "FPS 144 · ЦП 5% · ОЗУ 8,5/16 ГБ");
        assert!(line_with_fps(&t, None, None, Some("FPS игры: нужен запуск от администратора")).starts_with("FPS игры: нужен"));
        assert!(line(&s, None).chars().count() < 60, "one small line");
    }

    #[test]
    fn a_sample_can_be_taken_on_any_system() {
        let s = sample();
        assert!(s.cpu_pct <= 100);
    }
}
