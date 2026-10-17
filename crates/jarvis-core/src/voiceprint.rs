//! «Мой голос»: the program can listen only to the voice of its owner (an experimental, simple protection).
//!
//! How it works. From an utterance (16-bit mono audio) the program takes a small «voiceprint»: the average and the spread of 12 spectral
//! numbers (MFCC, the shape of the voice) of the frames where somebody speaks, and the usual height of the voice (the pitch). To
//! record «my voice» the user says five phrases; the program keeps the average of the five prints (numbers only — no audio is kept)
//! and a threshold made from how much the five differ from each other. Later every utterance is compared with that average: a voice that
//! is farther than the threshold (another person, a game, a film, the voice of Jarvis himself) is ignored.
//!
//! What it is NOT: it is not a biometric lock. It tells voices apart by their colour and height, it can be wrong on similar voices and in
//! a noisy room, and it does not protect against a recording of the owner's voice. That is why it is switched off by default, shows
//! every refusal with the distance and the threshold, and has a strictness setting. An utterance that is too short to judge (a one-word
//! command) is let through.
//!
//! The print lies in `voiceprint.json` of the data folder, on this computer only; «Удалить мой голос» removes it.

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::{json, Value};

pub const DIM: usize = 26;
// phrases the user is asked to say when recording the voice
pub const PROMPTS: [&str; 5] = ["Джарвис, открой браузер", "Включи игровой режим", "Иди вперёд, стой, смени оружие", "Который сейчас час", "Поставь таймер на пять минут"];
const MIN_VOICED_FRAMES: usize = 50;

// ---------------------------------------------------------------------------------------------
// the arithmetic: FFT, mel filters, MFCC, pitch
// ---------------------------------------------------------------------------------------------
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    // bit reversal
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f32::consts::PI / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let ncr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = ncr;
            }
        }
        len <<= 1;
    }
}

fn hz_to_mel(f: f32) -> f32 {
    2595.0 * (1.0 + f / 700.0).log10()
}

fn mel_to_hz(m: f32) -> f32 {
    700.0 * (10f32.powf(m / 2595.0) - 1.0)
}

// triangular filters over the bins of the spectrum: (first bin, weights)
fn mel_bank(nfft: usize, sr: u32, filters: usize) -> Vec<(usize, Vec<f32>)> {
    let top = (sr as f32 / 2.0).min(6500.0);
    let (lo, hi) = (hz_to_mel(100.0), hz_to_mel(top));
    let points: Vec<f32> = (0..filters + 2).map(|i| mel_to_hz(lo + (hi - lo) * i as f32 / (filters + 1) as f32)).collect();
    let bin = |f: f32| (f * nfft as f32 / sr as f32).floor() as usize;
    (0..filters)
        .map(|i| {
            let (a, b, c) = (bin(points[i]), bin(points[i + 1]).max(bin(points[i]) + 1), bin(points[i + 2]).max(bin(points[i + 1]) + 2));
            let weights: Vec<f32> = (a..=c).map(|k| if k <= b { (k - a) as f32 / (b - a) as f32 } else { (c - k) as f32 / (c - b) as f32 }).collect();
            (a, weights)
        })
        .collect()
}

// the voiceprint of an utterance: [mean of MFCC 1..12, spread of MFCC 1..12, ln pitch, spread of ln pitch]; the pitch is NaN when
// the voice had no clear pitch. None: too little speech to judge (under half a second of voice).
pub fn embed(samples: &[i16], sr: u32) -> Option<Vec<f32>> {
    if sr < 8000 || samples.len() < sr as usize / 2 {
        return None;
    }
    let frame = (sr as usize * 25 / 1000).max(64);
    let hop = (sr as usize * 10 / 1000).max(16);
    let nfft = frame.next_power_of_two();
    let x: Vec<f32> = samples.iter().map(|s| *s as f32 / 32768.0).collect();
    let window: Vec<f32> = (0..frame).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (frame - 1) as f32).cos()).collect();
    // the energy of every frame: the frames of speech are the loud ones
    let starts: Vec<usize> = (0..).map(|i| i * hop).take_while(|s| s + frame <= x.len()).collect();
    let energy: Vec<f32> = starts.iter().map(|&s| x[s..s + frame].iter().map(|v| v * v).sum::<f32>() / frame as f32).collect();
    let mut sorted = energy.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p90 = *sorted.get(sorted.len() * 9 / 10).unwrap_or(&0.0);
    let limit = (p90 * 0.06).max(1e-5);
    let voiced: Vec<usize> = (0..starts.len()).filter(|&i| energy[i] >= limit).collect();
    if voiced.len() < MIN_VOICED_FRAMES {
        return None;
    }
    let bank = mel_bank(nfft, sr, 26);
    let mut coeffs: Vec<[f32; 12]> = Vec::with_capacity(voiced.len());
    let mut logf0: Vec<f32> = Vec::new();
    for &i in &voiced {
        let s = starts[i];
        // pre-emphasis, the window, the spectrum
        let mut re = vec![0f32; nfft];
        let mut im = vec![0f32; nfft];
        for k in 0..frame {
            let prev = if s + k > 0 { x[s + k - 1] } else { 0.0 };
            re[k] = (x[s + k] - 0.97 * prev) * window[k];
        }
        fft(&mut re, &mut im);
        let power: Vec<f32> = (0..nfft / 2 + 1).map(|k| re[k] * re[k] + im[k] * im[k]).collect();
        let logmel: Vec<f32> = bank.iter().map(|(a, w)| (w.iter().enumerate().map(|(j, wj)| wj * power.get(a + j).copied().unwrap_or(0.0)).sum::<f32>() + 1e-10).ln()).collect();
        // the cosine transform: 12 numbers (the first, the loudness, is dropped)
        let mut c = [0f32; 12];
        for (m, cm) in c.iter_mut().enumerate() {
            *cm = logmel.iter().enumerate().map(|(j, l)| l * (std::f32::consts::PI * (m + 1) as f32 * (j as f32 + 0.5) / logmel.len() as f32).cos()).sum();
        }
        coeffs.push(c);
        // the pitch: the strongest repeat of the waveform between 60 and 400 Hz
        let seg = &x[s..s + frame];
        let r0: f32 = seg.iter().map(|v| v * v).sum();
        if r0 > 0.0 {
            let (lag_lo, lag_hi) = (sr as usize / 400, (sr as usize / 60).min(frame - 1));
            let mut best = (0.0f32, 0usize);
            for lag in lag_lo..=lag_hi {
                let r: f32 = (0..frame - lag).map(|k| seg[k] * seg[k + lag]).sum::<f32>() / r0;
                if r > best.0 {
                    best = (r, lag);
                }
            }
            if best.0 > 0.45 && best.1 > 0 {
                logf0.push((sr as f32 / best.1 as f32).ln());
            }
        }
    }
    let n = coeffs.len() as f32;
    let mut out = vec![0f32; DIM];
    for m in 0..12 {
        let mean = coeffs.iter().map(|c| c[m]).sum::<f32>() / n;
        let var = coeffs.iter().map(|c| (c[m] - mean).powi(2)).sum::<f32>() / n;
        out[m] = mean;
        out[12 + m] = var.sqrt();
    }
    if logf0.len() >= 8 {
        logf0.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median = logf0[logf0.len() / 2];
        let q1 = logf0[logf0.len() / 4];
        let q3 = logf0[logf0.len() * 3 / 4];
        out[24] = median;
        out[25] = q3 - q1;
    } else {
        out[24] = f32::NAN;
        out[25] = f32::NAN;
    }
    Some(out)
}

// how far two voiceprints are from each other (about 0.5 for the same speaker and 2 or more for different ones on the usual phrases)
pub fn distance(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != DIM || b.len() != DIM {
        return f32::INFINITY;
    }
    let mut sum = 0.0f32;
    for m in 0..12 {
        let w_mean = 1.0 / (6.0 - 0.25 * m as f32);
        sum += ((a[m] - b[m]) * w_mean).powi(2);
        sum += ((a[12 + m] - b[12 + m]) / 3.0).powi(2);
    }
    let mut d = (sum / 24.0).sqrt();
    // the pitch counts when both voices had one: a tenth part of the height is about one unit
    if a[24].is_finite() && b[24].is_finite() {
        let dp = (a[24] - b[24]).abs() / 0.12;
        d = (d * d + dp * dp / 4.0).sqrt();
    }
    d
}

// the average of several voiceprints (the pitch only of those that have one)
pub fn centroid(list: &[Vec<f32>]) -> Vec<f32> {
    let mut out = vec![0f32; DIM];
    for k in 0..DIM {
        let vals: Vec<f32> = list.iter().map(|v| v[k]).filter(|x| x.is_finite()).collect();
        out[k] = if vals.is_empty() { f32::NAN } else { vals.iter().sum::<f32>() / vals.len() as f32 };
    }
    out
}

// the threshold from the samples of the owner: each sample against the average of the others (a new phrase is always a little farther
// than the ones that made the average), with room to spare. The floor keeps it from being so small that the owner is refused.
pub fn threshold(samples: &[Vec<f32>]) -> f32 {
    if samples.len() < 3 {
        return 1.5;
    }
    let loo: Vec<f32> = (0..samples.len())
        .map(|i| {
            let others: Vec<Vec<f32>> = samples.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, v)| v.clone()).collect();
            distance(&samples[i], &centroid(&others))
        })
        .collect();
    let max = loo.iter().cloned().fold(0.0f32, f32::max);
    let mean = loo.iter().sum::<f32>() / loo.len() as f32;
    let sd = (loo.iter().map(|d| (d - mean).powi(2)).sum::<f32>() / loo.len() as f32).sqrt();
    (max * 1.6).max(mean + 3.0 * sd).max(0.9).min(3.0)
}

// ---------------------------------------------------------------------------------------------
// the print of the owner, its file, and what the program shows
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Debug, PartialEq)]
pub struct Print {
    pub centroid: Vec<f32>,
    pub threshold: f32,
    pub samples: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    pub print: Option<Print>,
    // «слушать только мой голос»
    pub enabled: bool,
    // 0 = soft (the threshold ×1.4), 1 = usual, 2 = strict (×0.8)
    pub strictness: u8,
}

impl Default for Saved {
    fn default() -> Self {
        Saved { print: None, enabled: false, strictness: 1 }
    }
}

fn vec_json(v: &[f32]) -> Value {
    Value::Array(v.iter().map(|x| if x.is_finite() { json!(*x) } else { Value::Null }).collect())
}

pub fn to_json(s: &Saved) -> String {
    let print = s.print.as_ref().map(|p| json!({ "centroid": vec_json(&p.centroid), "threshold": p.threshold, "samples": p.samples }));
    serde_json::to_string_pretty(&json!({ "version": 1, "enabled": s.enabled, "strictness": s.strictness, "print": print })).unwrap_or_default()
}

pub fn parse(text: &str) -> Saved {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return Saved::default() };
    let print = v.get("print").filter(|p| !p.is_null()).and_then(|p| {
        let c: Vec<f32> = p.get("centroid")?.as_array()?.iter().map(|x| x.as_f64().map(|f| f as f32).unwrap_or(f32::NAN)).collect();
        let t = p.get("threshold")?.as_f64()? as f32;
        (c.len() == DIM && t.is_finite() && t > 0.0).then(|| Print { centroid: c, threshold: t.min(5.0), samples: p.get("samples").and_then(|s| s.as_u64()).unwrap_or(0) as usize })
    });
    Saved { enabled: v.get("enabled").and_then(|e| e.as_bool()).unwrap_or(false) && print.is_some(), strictness: v.get("strictness").and_then(|s| s.as_u64()).unwrap_or(1).min(2) as u8, print }
}

fn file() -> std::path::PathBuf {
    #[cfg(test)]
    {
        return std::env::temp_dir().join(format!("jarvis-voiceprint-{}.json", std::process::id()));
    }
    #[cfg(not(test))]
    {
        crate::APP_DIR.join("voiceprint.json")
    }
}

fn load() -> Saved {
    std::fs::read_to_string(file()).map(|t| parse(&t)).unwrap_or_default()
}

fn save(s: &Saved) {
    let path = file();
    if let Ok(old) = std::fs::read_to_string(&path) {
        crate::keep_copy_if_repaired(&path, &old, &to_json(&parse(&old)));
    }
    let _ = crate::write_atomic(&path, to_json(s));
}

#[derive(Default)]
struct Live {
    // recording the voice: the prints made so far
    enrolling: Option<Vec<Vec<f32>>>,
    // «проверить мой голос»: the next utterance is only measured
    testing: bool,
    last: Option<(bool, f32, f32)>,
    accepted: u64,
    rejected: u64,
    // refusals in a row (the window tells the user when the owner seems to be refused again and again)
    rejected_in_row: u32,
    note: String,
}

static LIVE: Lazy<Mutex<Live>> = Lazy::new(|| Mutex::new(Live::default()));
static WRITE: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

// the utterance is only listened to for the voiceprint (a recording or a test): it must not run as a command
pub fn capturing() -> bool {
    let l = LIVE.lock();
    l.enrolling.is_some() || l.testing
}

// the lock is on: a print exists and the user switched it on
pub fn is_active() -> bool {
    let s = load();
    s.enabled && s.print.is_some()
}

pub fn effective_threshold(print: &Print, strictness: u8) -> f32 {
    print.threshold * match strictness {
        0 => 1.4,
        2 => 0.8,
        _ => 1.0,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    // the lock is off, or the utterance was too short to judge: let through
    Pass,
    Accepted { distance: f32 },
    Refused { distance: f32, threshold: f32 },
}

impl Verdict {
    pub fn allows(&self) -> bool {
        !matches!(self, Verdict::Refused { .. })
    }
}

// the utterance against the print of the owner
pub fn check(samples: &[i16], sr: u32) -> Verdict {
    let s = load();
    let Some(print) = s.print.as_ref().filter(|_| s.enabled) else { return Verdict::Pass };
    let Some(e) = embed(samples, sr) else { return Verdict::Pass };
    let d = distance(&e, &print.centroid);
    let t = effective_threshold(print, s.strictness);
    let mut l = LIVE.lock();
    l.last = Some((d <= t, d, t));
    if d <= t {
        l.accepted += 1;
        l.rejected_in_row = 0;
        Verdict::Accepted { distance: d }
    } else {
        l.rejected += 1;
        l.rejected_in_row += 1;
        Verdict::Refused { distance: d, threshold: t }
    }
}

// how many refusals in a row there were (the caller tells the user at the third one)
pub fn refusals_in_a_row() -> u32 {
    LIVE.lock().rejected_in_row
}

pub fn start_enrollment() {
    let mut l = LIVE.lock();
    l.enrolling = Some(Vec::new());
    l.testing = false;
    l.note = format!("Скажите фразу 1 из {}: «{}»", PROMPTS.len(), PROMPTS[0]);
}

pub fn cancel_enrollment() {
    let mut l = LIVE.lock();
    l.enrolling = None;
    l.testing = false;
    l.note = "Запись голоса отменена.".into();
}

pub fn start_test() {
    let mut l = LIVE.lock();
    if l.enrolling.is_none() {
        l.testing = true;
        l.note = "Скажите любую фразу: я покажу, принял бы я её.".into();
    }
}

// one utterance while recording or testing: the answer for the window. The utterance does not run as a command.
pub fn take_capture(samples: &[i16], sr: u32) -> String {
    let mut l = LIVE.lock();
    let e = embed(samples, sr);
    if l.testing {
        l.testing = false;
        let s = load();
        let msg = match (e, s.print.as_ref()) {
            (None, _) => "Фраза слишком короткая или тихая: скажите длиннее, 2–3 секунды.".to_string(),
            (Some(_), None) => "Голос ещё не записан: нажмите «Записать мой голос».".to_string(),
            (Some(e), Some(p)) => {
                let (d, t) = (distance(&e, &p.centroid), effective_threshold(p, s.strictness));
                l.last = Some((d <= t, d, t));
                format!("{}: расстояние {:.2}, порог {:.2}.", if d <= t { "Принял бы" } else { "Не принял бы" }, d, t)
            }
        };
        l.note = msg.clone();
        return msg;
    }
    let Some(list) = l.enrolling.as_mut() else { return String::new() };
    let Some(e) = e else {
        let msg = "Фраза слишком короткая или тихая, не засчитана. Скажите её целиком, громко и чётко.".to_string();
        l.note = msg.clone();
        return msg;
    };
    list.push(e);
    let n = list.len();
    if n < PROMPTS.len() {
        let msg = format!("Записано {} из {}. Скажите фразу {}: «{}»", n, PROMPTS.len(), n + 1, PROMPTS[n]);
        l.note = msg.clone();
        return msg;
    }
    // all of them: the average and the threshold
    let list = l.enrolling.take().unwrap_or_default();
    let print = Print { centroid: centroid(&list), threshold: threshold(&list), samples: list.len() };
    let msg = format!("Голос записан (порог {:.2}). Включите «Слушать только мой голос» и проверьте кнопкой «Проверить».", print.threshold);
    let _w = WRITE.lock();
    let mut saved = load();
    saved.print = Some(print);
    save(&saved);
    l.note = msg.clone();
    msg
}

pub fn set_enabled(on: bool) -> Result<(), String> {
    let _w = WRITE.lock();
    let mut s = load();
    if on && s.print.is_none() {
        return Err("Сначала запишите свой голос.".into());
    }
    s.enabled = on;
    save(&s);
    LIVE.lock().rejected_in_row = 0;
    Ok(())
}

pub fn set_strictness(level: u8) {
    let _w = WRITE.lock();
    let mut s = load();
    s.strictness = level.min(2);
    save(&s);
}

// the print is removed (the file too); the program listens to everybody again
pub fn delete() {
    let _w = WRITE.lock();
    let _ = std::fs::remove_file(file());
    let mut l = LIVE.lock();
    *l = Live::default();
    l.note = "Голос удалён: Jarvis слушает всех.".into();
}

pub fn status_json() -> Value {
    let s = load();
    let l = LIVE.lock();
    let step = l.enrolling.as_ref().map(|e| e.len()).unwrap_or(0);
    json!({
        "enrolled": s.print.is_some(), "enabled": s.enabled && s.print.is_some(), "strictness": s.strictness,
        "enrolling": l.enrolling.is_some(), "testing": l.testing, "step": step, "total": PROMPTS.len(),
        "prompt": if l.enrolling.is_some() { PROMPTS.get(step).copied().unwrap_or("") } else { "" },
        "note": l.note, "accepted": l.accepted, "rejected": l.rejected,
        "last": l.last.map(|(ok, d, t)| json!({ "ok": ok, "distance": d, "threshold": t })),
        "threshold": s.print.as_ref().map(|p| effective_threshold(p, s.strictness))
    })
}

#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 16_000;

    // a small «voice» made by the source-and-filter way: pulses of the pitch `f0`, three resonances (the formants) that change with the
    // sounds, a little noise; `seed` makes another «phrase»
    fn voice(f0: f32, formant_scale: f32, seed: u64, secs: f32) -> Vec<i16> {
        let mut rng = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let mut rand = move || {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((rng >> 33) as f32) / (u32::MAX >> 1) as f32
        };
        let sounds: [(f32, f32, f32); 5] = [(700.0, 1220.0, 2600.0), (300.0, 2200.0, 3000.0), (320.0, 800.0, 2500.0), (500.0, 1800.0, 2500.0), (500.0, 900.0, 2400.0)];
        let n = (SR as f32 * secs) as usize;
        let mut out = Vec::with_capacity(n);
        let (mut phase, mut pitch) = (0.0f32, f0);
        let mut state = [[0.0f32; 2]; 3];
        let mut left = 0usize;
        let mut cur = sounds[0];
        let mut amp = 1.0f32;
        for i in 0..n {
            if left == 0 {
                cur = sounds[(rand() * 5.0) as usize % 5];
                left = (SR as f32 * (0.12 + 0.12 * rand())) as usize;
                amp = if rand() < 0.15 { 0.0 } else { 0.6 + 0.4 * rand() };
            }
            left -= 1;
            // the pitch wanders a little
            pitch = (pitch + (rand() - 0.5) * 0.4).clamp(f0 * 0.92, f0 * 1.08);
            phase += pitch / SR as f32;
            if phase >= 1.0 {
                phase -= 1.0;
            }
            let glottal = (1.0 - phase) * (1.0 - phase) * 2.0 - 0.6;
            let mut y = glottal * amp;
            // three resonators
            let formants = [cur.0, cur.1, cur.2];
            let mut sum = 0.0;
            for (k, f) in formants.iter().enumerate() {
                let f = f * formant_scale;
                let r = 0.96;
                let (a1, a2) = (2.0 * r * (2.0 * std::f32::consts::PI * f / SR as f32).cos(), -r * r);
                let v = y + a1 * state[k][0] + a2 * state[k][1];
                state[k][1] = state[k][0];
                state[k][0] = v;
                sum += v * (1.0 - 0.25 * k as f32);
            }
            y = sum * 0.05 + (rand() - 0.5) * 0.004;
            out.push((y.clamp(-1.0, 1.0) * 12000.0) as i16);
        }
        out
    }

    fn noise(secs: f32, seed: u64) -> Vec<i16> {
        let mut rng = seed;
        (0..(SR as f32 * secs) as usize)
            .map(|_| {
                rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                (((rng >> 40) as i32 % 6000) - 3000) as i16
            })
            .collect()
    }

    #[test]
    fn the_fft_finds_the_frequency_of_a_tone_and_the_mel_bank_covers_the_spectrum() {
        let n = 512;
        let mut re: Vec<f32> = (0..n).map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / SR as f32).sin()).collect();
        let mut im = vec![0.0; n];
        fft(&mut re, &mut im);
        let peak = (0..n / 2).max_by(|a, b| (re[*a].powi(2) + im[*a].powi(2)).partial_cmp(&(re[*b].powi(2) + im[*b].powi(2))).unwrap()).unwrap();
        assert!((peak as f32 * SR as f32 / n as f32 - 1000.0).abs() < 40.0, "the peak is at {} Hz", peak as f32 * SR as f32 / n as f32);
        let bank = mel_bank(512, SR, 26);
        assert_eq!(bank.len(), 26);
        assert!(bank.iter().all(|(_, w)| !w.is_empty() && w.iter().cloned().fold(0.0, f32::max) > 0.0));
        assert!(bank.windows(2).all(|w| w[0].0 <= w[1].0), "the filters go up in frequency");
    }

    #[test]
    fn a_print_is_made_only_of_enough_speech_and_has_the_pitch_of_the_voice() {
        assert!(embed(&[], SR).is_none());
        assert!(embed(&vec![0i16; 32_000], SR).is_none(), "silence has no voice");
        assert!(embed(&voice(110.0, 1.0, 1, 0.3), SR).is_none(), "a third of a second is too little");
        let e = embed(&voice(110.0, 1.0, 1, 2.5), SR).expect("a print");
        assert_eq!(e.len(), DIM);
        // the pitch of the synthetic voice is near 110 Hz
        assert!(e[24].is_finite() && (e[24].exp() - 110.0).abs() < 15.0, "pitch {}", e[24].exp());
        let high = embed(&voice(210.0, 1.0, 1, 2.5), SR).unwrap();
        assert!((high[24].exp() - 210.0).abs() < 25.0, "pitch {}", high[24].exp());
    }

    #[test]
    fn the_same_voice_in_other_phrases_is_close_and_another_voice_and_noise_are_far() {
        let owner: Vec<Vec<f32>> = (1..=5).map(|s| embed(&voice(110.0, 1.0, s, 2.5), SR).unwrap()).collect();
        let c = centroid(&owner);
        let t = threshold(&owner);
        assert!((0.9..=3.0).contains(&t), "threshold {}", t);
        // a new phrase of the owner: inside
        for seed in [11u64, 12, 13, 14] {
            let d = distance(&embed(&voice(110.0, 1.0, seed, 2.0), SR).unwrap(), &c);
            assert!(d <= t, "the owner is refused: {} > {}", d, t);
        }
        // a high voice with other resonances (a woman, a child, a game character): outside
        for (f0, scale, seed) in [(210.0f32, 1.18f32, 21u64), (190.0, 1.12, 22), (260.0, 1.3, 23)] {
            let d = distance(&embed(&voice(f0, scale, seed, 2.5), SR).unwrap(), &c);
            assert!(d > t, "another voice ({} Hz) is accepted: {} <= {}", f0, d, t);
        }
        // noise is not the owner (or it has no print at all)
        match embed(&noise(2.5, 5), SR) {
            None => {}
            Some(e) => assert!(distance(&e, &c) > t, "noise is accepted"),
        }
    }

    #[test]
    fn the_threshold_does_not_grow_without_limit_and_never_shrinks_to_nothing() {
        let same = vec![embed(&voice(110.0, 1.0, 1, 2.5), SR).unwrap(); 5];
        assert!(threshold(&same) >= 0.9, "five equal prints: the floor");
        assert!(threshold(&same[..2]) >= 1.0, "too few samples: a fixed value");
        let mut wild = same.clone();
        wild[0] = embed(&voice(300.0, 1.4, 9, 2.5), SR).unwrap();
        wild[1] = embed(&voice(90.0, 0.8, 8, 2.5), SR).unwrap();
        assert!(threshold(&wild) <= 3.0);
        assert!(distance(&[0.0; 3], &same[0]).is_infinite(), "prints of other sizes are never near");
    }

    #[test]
    fn the_print_file_round_trips_without_audio_and_a_broken_file_gives_nothing() {
        let e = embed(&voice(110.0, 1.0, 1, 2.5), SR).unwrap();
        let s = Saved { print: Some(Print { centroid: e.clone(), threshold: 1.2, samples: 5 }), enabled: true, strictness: 2 };
        let text = to_json(&s);
        assert!(!text.contains("samples\": ["), "numbers only, no audio");
        assert_eq!(parse(&text), s);
        // a missing pitch (NaN) is written as null and read back as missing
        let mut nan = e.clone();
        nan[24] = f32::NAN;
        nan[25] = f32::NAN;
        let back = parse(&to_json(&Saved { print: Some(Print { centroid: nan, threshold: 1.0, samples: 3 }), enabled: false, strictness: 1 }));
        assert!(back.print.unwrap().centroid[24].is_nan());
        assert_eq!(parse("не json"), Saved::default());
        // the lock cannot be on without a print
        assert!(!parse(r#"{"enabled":true}"#).enabled);
        assert_eq!(parse(r#"{"print":{"centroid":[1,2],"threshold":1}}"#).print, None, "a print of the wrong size is not taken");
    }

    // recording the voice, checking, strictness, deleting: the whole way with the file
    #[test]
    fn the_voice_is_recorded_in_five_phrases_the_lock_refuses_other_voices_and_the_print_can_be_removed() {
        let _l = TEST_LOCK.lock();
        delete();
        assert!(!is_active() && !capturing());
        assert!(set_enabled(true).is_err(), "no print yet");
        start_enrollment();
        assert!(capturing());
        // a phrase that is too short is not counted
        assert!(take_capture(&voice(110.0, 1.0, 1, 0.3), SR).contains("слишком короткая"));
        assert_eq!(status_json()["step"], 0);
        for s in 1..=5u64 {
            let msg = take_capture(&voice(110.0, 1.0, s, 2.5), SR);
            if s < 5 {
                assert!(msg.contains(&format!("Записано {} из 5", s)), "{}", msg);
            } else {
                assert!(msg.contains("Голос записан"), "{}", msg);
            }
        }
        assert!(!capturing() && status_json()["enrolled"] == true);
        // not on yet: everybody is let through
        assert_eq!(check(&voice(210.0, 1.2, 40, 2.5), SR), Verdict::Pass);
        set_enabled(true).unwrap();
        assert!(is_active());
        assert!(matches!(check(&voice(110.0, 1.0, 31, 2.5), SR), Verdict::Accepted { .. }), "the owner");
        let other = check(&voice(210.0, 1.2, 41, 2.5), SR);
        assert!(matches!(other, Verdict::Refused { .. }) && !other.allows(), "another voice: {:?}", other);
        assert_eq!(refusals_in_a_row(), 1);
        // too short to judge: let through
        assert_eq!(check(&voice(210.0, 1.2, 42, 0.3), SR), Verdict::Pass);
        // a test of the voice measures and runs nothing
        start_test();
        assert!(capturing());
        assert!(take_capture(&voice(110.0, 1.0, 51, 2.5), SR).starts_with("Принял бы"));
        assert!(!capturing());
        // strictness changes the threshold
        let soft = effective_threshold(load().print.as_ref().unwrap(), 0);
        let strict = effective_threshold(load().print.as_ref().unwrap(), 2);
        assert!(soft > strict);
        set_strictness(2);
        assert_eq!(status_json()["strictness"], 2);
        // removed: everybody again, the file is gone
        delete();
        assert!(!is_active() && !file().exists());
        assert_eq!(check(&voice(210.0, 1.2, 43, 2.5), SR), Verdict::Pass);
    }
}
