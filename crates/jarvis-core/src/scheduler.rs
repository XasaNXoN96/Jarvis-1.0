//! The scheduler of the user's tasks: «каждый день в 9:00 открой почту», «завтра в 8:30 напомни …», «каждые 30 минут …».
//! The jobs live in a file and survive a restart. A job is CLAIMED (its next time and last run are written) BEFORE its phrase
//! is sent, so a crash or a second tick never runs it twice. A run missed while Jarvis was off is run once if it is not older than
//! the window of the job, otherwise skipped (never a burst of catch-up runs). The phrase goes through the normal command door
//! (`remote::submit`, door «script»), so every check of the commands applies to it.

use crate::{store, textnorm};
use chrono::{Datelike, Duration, NaiveDateTime, NaiveTime, Timelike, Weekday};
use serde::{Deserialize, Serialize};

const FILE: &str = "scheduler.json";
pub const MAX_JOBS: usize = 100;
// a missed run is still made up for within this time
pub const MISSED_WINDOW: u64 = 3600;
pub const MIN_EVERY: u64 = 60;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind", content = "secs")]
pub enum Repeat {
    Once,
    Every(u64),
    Daily,
    Weekly,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Job {
    pub id: String,
    // the phrase said to Jarvis when the time comes
    pub text: String,
    // the next run, unix seconds (UTC)
    pub at: u64,
    pub repeat: Repeat,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub last_run: u64,
    // how many times it ran (the window shows it)
    #[serde(default)]
    pub runs: u32,
    // the door that asked for this job ("phone" / "telegram" when it was a remote one, else ""): the job runs through the same door, so a job
    // made from the phone is not a way to get the trust of a script (origin.rs). A job saved by an older version has none.
    #[serde(default)]
    pub door: String,
}
fn yes() -> bool {
    true
}

#[derive(Debug, PartialEq)]
pub struct Due {
    pub id: String,
    // the time was missed (Jarvis was off) and the job runs late
    pub late: bool,
}

pub fn list() -> Vec<Job> {
    store::read::<Vec<Job>>(FILE).0
}
fn save(j: &[Job]) -> Result<(), String> {
    store::write(FILE, &j)
}

pub fn add(text: &str, at: u64, repeat: Repeat) -> Result<Job, String> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > 300 {
        return Err("Фраза задания пустая или слишком длинная.".into());
    }
    if let Repeat::Every(s) = repeat {
        if s < MIN_EVERY {
            return Err("Чаще раза в минуту повторять нельзя.".into());
        }
    }
    let mut jobs = list();
    if jobs.len() >= MAX_JOBS {
        return Err("Слишком много заданий.".into());
    }
    let door = match crate::origin::current() {
        Some(s) if s.is_remote() => s.as_str().to_string(),
        _ => String::new(),
    };
    let job = Job { id: store::new_id(), text: text.to_string(), at, repeat, enabled: true, last_run: 0, runs: 0, door };
    jobs.push(job.clone());
    save(&jobs)?;
    Ok(job)
}

pub fn remove(id: &str) -> bool {
    let mut j = list();
    let n = j.len();
    j.retain(|x| x.id != id);
    j.len() != n && save(&j).is_ok()
}

// Changes a job: a new phrase with a time («каждый день в 10:00 открой почту») replaces its text, its time and its repeat. The job keeps its
// number, its switch, its count of runs and the door that asked for it (a change cannot turn a phone job into a job with more trust).
// Returns (the job as it was, the job as it is).
pub fn edit(id: &str, phrase: &str, now: NaiveDateTime, offset: i64) -> Result<(Job, Job), String> {
    let plan = parse(phrase, now, offset).ok_or("Не понял время. Примеры: «каждый день в 10:00 открой почту», «завтра в 8:30 напомни про встречу».")?;
    let text = plan.text.trim();
    if text.is_empty() || text.chars().count() > 300 {
        return Err("Фраза задания пустая или слишком длинная.".into());
    }
    if let Repeat::Every(s) = plan.repeat {
        if s < MIN_EVERY {
            return Err("Чаще раза в минуту повторять нельзя.".into());
        }
    }
    let mut jobs = list();
    let j = jobs.iter_mut().find(|x| x.id == id).ok_or("Такого задания нет.")?;
    let old = j.clone();
    j.text = text.to_string();
    j.at = plan.at;
    j.repeat = plan.repeat;
    // a one-time job that already ran was switched off by its run: editing it sets it up again (a job the user switched off by hand stays off)
    if old.repeat == Repeat::Once && old.runs > 0 {
        j.enabled = true;
    }
    let new = j.clone();
    save(&jobs)?;
    Ok((old, new))
}

// puts a job back as it was (the undo of an edit); only while the job is still as the edit left it
pub fn restore(old: &Job, expect_now: &Job) -> Result<(), String> {
    let mut jobs = list();
    let j = jobs.iter_mut().find(|x| x.id == old.id).ok_or("Задания уже нет.")?;
    if j.text != expect_now.text || j.at != expect_now.at || j.repeat != expect_now.repeat {
        return Err("Задание уже менялось после этой правки: вернуть старое нельзя.".into());
    }
    j.text = old.text.clone();
    j.at = old.at;
    j.repeat = old.repeat;
    j.enabled = old.enabled;
    save(&jobs)
}

pub fn set_enabled(id: &str, on: bool) -> bool {
    let mut j = list();
    match j.iter_mut().find(|x| x.id == id) {
        Some(x) => {
            x.enabled = on;
            save(&j).is_ok()
        }
        None => false,
    }
}

fn step(repeat: Repeat) -> Option<u64> {
    match repeat {
        Repeat::Once => None,
        Repeat::Every(s) => Some(s.max(MIN_EVERY)),
        Repeat::Daily => Some(86400),
        Repeat::Weekly => Some(604800),
    }
}

// the first time after `now` of a repeating job that was due at `at`
pub fn next_after(at: u64, repeat: Repeat, now: u64) -> Option<u64> {
    let s = step(repeat)?;
    if at > now {
        return Some(at);
    }
    Some(at + ((now - at) / s + 1) * s)
}

// the jobs to run now. Pure: nothing is written.
pub fn due(jobs: &[Job], now: u64) -> Vec<Due> {
    jobs.iter().filter(|j| j.enabled && j.at <= now).map(|j| Due { id: j.id.clone(), late: now - j.at > 90 }).collect()
}

// writes the new state of a due job BEFORE it runs. Returns the phrase to run, or None when the run is skipped (too old / not due / gone).
pub fn claim(id: &str, now: u64) -> Option<String> {
    claim_with_door(id, now).map(|(t, _)| t)
}

// the same, with the door of the job
pub fn claim_with_door(id: &str, now: u64) -> Option<(String, String)> {
    let mut jobs = list();
    let i = jobs.iter().position(|j| j.id == id)?;
    let j = &mut jobs[i];
    if !j.enabled || j.at > now {
        return None;
    }
    let too_old = now - j.at > MISSED_WINDOW;
    let text = j.text.clone();
    let door = j.door.clone();
    match next_after(j.at, j.repeat, now) {
        Some(n) => j.at = n,
        None => j.enabled = false,
    }
    if !too_old {
        j.last_run = now;
        j.runs += 1;
    }
    // (if the state cannot be written the job does not run: better a missed run than a repeated one)
    save(&jobs).ok()?;
    if too_old { None } else { Some((text, door)) }
}

// the door a job runs through: the remote one it came from, else «script»
pub fn source_of(door: &str) -> crate::origin::Source {
    match door {
        "phone" => crate::origin::Source::Phone,
        "telegram" => crate::origin::Source::Telegram,
        _ => crate::origin::Source::Script,
    }
}

// one pass: every due job is claimed and its phrase handed to `run`; returns the phrases that were run
pub fn tick(now: u64, run: &mut dyn FnMut(&str)) -> Vec<String> {
    tick_with_door(now, &mut |t, _| run(t))
}

pub fn tick_with_door(now: u64, run: &mut dyn FnMut(&str, &str)) -> Vec<String> {
    let mut done = Vec::new();
    for d in due(&list(), now) {
        if let Some((text, door)) = claim_with_door(&d.id, now) {
            run(&text, &door);
            done.push(text);
        }
    }
    done
}

pub fn start() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_secs(15));
        tick_with_door(store::now(), &mut |t, door| {
            log::info!("Scheduler runs a job ({} chars)", t.chars().count());
            crate::audit::record_door(if door.is_empty() { "script" } else { door }, "scheduler", "job_run", "", "allowed", false, "задание запущено по расписанию");
            crate::remote::submit(source_of(door), t);
        });
    });
}

// ---------- the phrase ----------

fn weekday(w: &str) -> Option<Weekday> {
    Some(match textnorm::normalize(w).as_str() {
        "понедельник" | "monday" | "dushanba" => Weekday::Mon,
        "вторник" | "tuesday" | "seshanba" => Weekday::Tue,
        "среду" | "среда" | "wednesday" | "chorshanba" => Weekday::Wed,
        "четверг" | "thursday" | "payshanba" => Weekday::Thu,
        "пятницу" | "пятница" | "friday" | "juma" => Weekday::Fri,
        "субботу" | "суббота" | "saturday" | "shanba" => Weekday::Sat,
        "воскресенье" | "sunday" | "yakshanba" => Weekday::Sun,
        _ => return None,
    })
}

// «9:00», «в 18.30», «at 9:05»
fn clock(words: &[&str]) -> Option<(NaiveTime, usize)> {
    for (i, w) in words.iter().enumerate() {
        let w = w.trim_matches(|c: char| !c.is_ascii_digit() && c != ':' && c != '.');
        let (h, m) = match w.split_once([':', '.']) {
            Some((h, m)) => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
            None => continue,
        };
        if let Some(t) = NaiveTime::from_hms_opt(h, m, 0) {
            return Some((t, i));
        }
    }
    None
}

pub struct Plan {
    pub text: String,
    pub at: u64,
    pub repeat: Repeat,
}

// `now` is the local time, `offset` is local minus UTC in seconds.
// «каждый день в 9:00 открой почту» / «каждую пятницу в 18:00 …» / «каждые 30 минут …» / «завтра в 8:30 …» / «в 18:30 …» / «every day at 9:00 …»
pub fn parse(text: &str, now: NaiveDateTime, offset: i64) -> Option<Plan> {
    let t = text.trim();
    let words: Vec<&str> = t.split_whitespace().collect();
    if words.len() < 3 {
        return None;
    }
    let low: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
    let l: Vec<&str> = low.iter().map(|s| s.as_str()).collect();
    let to_utc = |dt: NaiveDateTime| (dt.and_utc().timestamp() - offset).max(0) as u64;
    let rest_after = |idx: usize| words[idx + 1..].join(" ");
    // the pieces
    let (mut repeat, mut date_shift, mut wd): (Option<Repeat>, Option<i64>, Option<Weekday>) = (None, None, None);
    let text_from;
    let mut every_secs: Option<u64> = None;
    match l[0] {
        "каждый" | "каждую" | "каждое" | "каждые" | "every" | "har" => {
            let w1 = l.get(1).copied().unwrap_or("");
            if matches!(w1, "день" | "day" | "kuni" | "kun") {
                repeat = Some(Repeat::Daily);
                text_from = 2;
            } else if let Some(d) = weekday(w1) {
                repeat = Some(Repeat::Weekly);
                wd = Some(d);
                text_from = 2;
            } else if let Ok(n) = w1.parse::<u64>() {
                let unit = l.get(2).copied().unwrap_or("");
                let mult = if unit.starts_with("мин") || unit.starts_with("min") { 60 } else if unit.starts_with("час") || unit.starts_with("hour") || unit.starts_with("soat") { 3600 } else { 0 };
                if mult == 0 {
                    return None;
                }
                every_secs = Some(n * mult);
                text_from = 3;
            } else if let Some(mult) = match w1 {
                "минуту" | "minute" => Some(60),
                "час" | "hour" => Some(3600),
                _ => None,
            } {
                every_secs = Some(mult);
                text_from = 2;
            } else {
                return None;
            }
        }
        "завтра" | "tomorrow" | "ertaga" => {
            date_shift = Some(1);
            text_from = 1;
        }
        "сегодня" | "today" | "bugun" => {
            date_shift = Some(0);
            text_from = 1;
        }
        "в" | "at" | "soat" => {
            date_shift = Some(0);
            text_from = 0;
        }
        _ => return None,
    }
    if let Some(s) = every_secs {
        let body = words[text_from..].join(" ");
        if body.is_empty() {
            return None;
        }
        let at = now.and_utc().timestamp() - offset + s as i64;
        return Some(Plan { text: body, at: at.max(0) as u64, repeat: Repeat::Every(s) });
    }
    // the time of the day
    let (time, ti) = clock(&words[text_from..].iter().copied().collect::<Vec<_>>()).map(|(t, i)| (t, i + text_from))?;
    // the words between the start and the time are only «в / at / soat»
    let body = rest_after(ti);
    let body = body.trim().to_string();
    if body.is_empty() {
        return None;
    }
    let today = now.date();
    let mut day = today + Duration::days(date_shift.unwrap_or(0));
    if let Some(d) = wd {
        while day.weekday() != d || day.and_time(time) <= now {
            day += Duration::days(1);
        }
    } else if date_shift == Some(0) && day.and_time(time) <= now && repeat.is_none() {
        // «в 18:30», already past today: tomorrow
        day += Duration::days(1);
    } else if repeat == Some(Repeat::Daily) && day.and_time(time) <= now {
        day += Duration::days(1);
    }
    let at = to_utc(day.and_time(time.with_second(0)?));
    if repeat.is_none() {
        repeat = Some(Repeat::Once);
    }
    let _ = (Datelike::year(&today), Timelike::hour(&time));
    Some(Plan { text: body, at, repeat: repeat? })
}

pub fn describe(j: &Job, offset: i64) -> String {
    let local = chrono::DateTime::from_timestamp(j.at as i64 + offset, 0).map(|d| d.naive_utc().format("%d.%m %H:%M").to_string()).unwrap_or_default();
    let r = match j.repeat {
        Repeat::Once => "один раз".to_string(),
        Repeat::Every(s) if s % 3600 == 0 => format!("каждые {} ч", s / 3600),
        Repeat::Every(s) => format!("каждые {} мин", s / 60),
        Repeat::Daily => "каждый день".into(),
        Repeat::Weekly => "каждую неделю".into(),
    };
    format!("{} — {} (следующий запуск {})", j.text, r, local)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    // a job keeps the door that asked for it: the phone cannot get the trust of a script by scheduling
    #[test]
    fn a_job_from_a_remote_door_runs_through_that_door_and_an_old_job_has_none() {
        store::with_temp(|| {
            let from_phone = crate::origin::run_as(crate::origin::Source::Phone, || add("открой блокнот", 100, Repeat::Once)).unwrap();
            let from_window = crate::origin::run_as(crate::origin::Source::Window, || add("открой почту", 100, Repeat::Once)).unwrap();
            assert_eq!((from_phone.door.as_str(), from_window.door.as_str()), ("phone", ""));
            assert_eq!(claim_with_door(&from_phone.id, 100), Some(("открой блокнот".to_string(), "phone".to_string())));
            assert_eq!(source_of("phone"), crate::origin::Source::Phone);
            assert_eq!(source_of("telegram"), crate::origin::Source::Telegram);
            assert_eq!(source_of(""), crate::origin::Source::Script);
            assert_eq!(source_of("../x"), crate::origin::Source::Script, "an unknown name is nobody special");
            // a file of an older version has no door
            let mut v = serde_json::to_value(&from_window).unwrap();
            v.as_object_mut().unwrap().remove("door");
            let old: Job = serde_json::from_value(v).unwrap();
            assert_eq!(old.door, "");
        });
    }

    #[test]
    fn a_job_can_be_edited_keeping_its_number_runs_and_door_and_the_edit_can_be_taken_back() {
        store::with_temp(|| {
            let j = crate::origin::run_as(crate::origin::Source::Phone, || add("открой почту", 100, Repeat::Once)).unwrap();
            claim_with_door(&j.id, 100);
            let (old, new) = edit(&j.id, "каждый день в 10:00 открой блокнот", t(8, 0), 0).unwrap();
            assert_eq!((old.text.as_str(), new.text.as_str()), ("открой почту", "открой блокнот"));
            assert!(!old.enabled && new.enabled, "a finished one-time job is set up again by an edit");
            assert_eq!(new.repeat, Repeat::Daily);
            let now = list().into_iter().find(|x| x.id == j.id).unwrap();
            assert_eq!((now.id.as_str(), now.runs, now.door.as_str(), now.enabled), (j.id.as_str(), 1, "phone", true), "number, runs, door and switch are kept");
            // a phrase without a time, an empty one, an unknown job and a too frequent repeat change nothing
            assert!(edit(&j.id, "открой почту", t(8, 0), 0).is_err());
            assert!(edit(&j.id, "", t(8, 0), 0).is_err());
            assert!(edit("nobody", "каждый день в 10:00 открой блокнот", t(8, 0), 0).is_err());
            assert_eq!(list().iter().find(|x| x.id == j.id).unwrap().text, "открой блокнот");
            // the undo puts the old one back, but not when the job has changed again
            restore(&old, &new).unwrap();
            let back = list().into_iter().find(|x| x.id == j.id).unwrap();
            assert_eq!((back.text.as_str(), back.repeat), ("открой почту", Repeat::Once));
            assert!(restore(&old, &new).is_err(), "the job is no longer as the edit left it");
        });
    }

    fn t(h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 22, ).unwrap().and_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn parses_the_phrases() {
        let off = 5 * 3600;
        let now = t(10, 0);
        let p = parse("каждый день в 9:00 открой почту", now, off).unwrap();
        assert_eq!((p.text.as_str(), p.repeat), ("открой почту", Repeat::Daily));
        // 9:00 already passed today → tomorrow 9:00 local = 04:00 UTC
        assert_eq!(p.at, (t(9, 0) + Duration::days(1)).and_utc().timestamp() as u64 - off as u64);
        let p = parse("в 18:30 позвонить маме", now, off).unwrap();
        assert_eq!((p.repeat, p.at), (Repeat::Once, t(18, 30).and_utc().timestamp() as u64 - off as u64));
        let p = parse("завтра в 8:15 напомни про встречу", now, off).unwrap();
        assert_eq!(p.at, (t(8, 15) + Duration::days(1)).and_utc().timestamp() as u64 - off as u64);
        let p = parse("каждые 30 минут пей воду", now, off).unwrap();
        assert_eq!(p.repeat, Repeat::Every(1800));
        let p = parse("каждую пятницу в 18:00 отчёт", now, off).unwrap();
        assert_eq!(p.repeat, Repeat::Weekly);
        // 2026-10-22 is a Thursday: the next Friday is the 23rd
        assert_eq!(p.at, t(18, 0).and_utc().timestamp() as u64 + 86400 - off as u64);
        assert!(parse("every day at 9:00 check mail", now, off).is_some());
        assert!(parse("открой браузер", now, off).is_none());
        assert!(parse("в 18:30", now, off).is_none());
    }

    #[test]
    fn next_time_never_bursts() {
        assert_eq!(next_after(1000, Repeat::Every(60), 1000 + 60 * 10 + 5), Some(1000 + 60 * 11));
        assert_eq!(next_after(1000, Repeat::Once, 5000), None);
        assert_eq!(next_after(5000, Repeat::Daily, 1000), Some(5000));
    }

    #[test]
    fn a_job_runs_once_and_not_twice() {
        store::with_temp(|| {
            let j = add("открой почту", 1000, Repeat::Once).unwrap();
            let mut ran = Vec::new();
            tick(1010, &mut |t| ran.push(t.to_string()));
            tick(1010, &mut |t| ran.push(t.to_string()));
            tick(2000, &mut |t| ran.push(t.to_string()));
            assert_eq!(ran, vec!["открой почту"]);
            let got = list().into_iter().find(|x| x.id == j.id).unwrap();
            assert!(!got.enabled);
            assert_eq!(got.runs, 1);
        });
    }

    #[test]
    fn a_missed_run_is_made_up_once_or_skipped() {
        store::with_temp(|| {
            // every 10 minutes, Jarvis was off for 5 hours: nothing runs (too old), the next time is in the future
            let j = add("пей воду", 1000, Repeat::Every(600)).unwrap();
            let now = 1000 + 5 * 3600 + 30;
            let mut ran = 0;
            tick(now, &mut |_| ran += 1);
            assert_eq!(ran, 0);
            let got = list().into_iter().find(|x| x.id == j.id).unwrap();
            assert!(got.at > now && got.at - now <= 600 && got.enabled);
            // 20 minutes late: one late run, not three
            let k = add("зарядка", 100_000, Repeat::Every(600)).unwrap();
            let mut ran2 = 0;
            tick(100_000 + 1200 + 5, &mut |_| ran2 += 1);
            assert_eq!(ran2, 1);
            assert_eq!(list().into_iter().find(|x| x.id == k.id).unwrap().runs, 1);
        });
    }

    #[test]
    fn disabled_and_removed_jobs_do_not_run() {
        store::with_temp(|| {
            let a = add("а б", 10, Repeat::Daily).unwrap();
            assert!(set_enabled(&a.id, false));
            let mut ran = 0;
            tick(100, &mut |_| ran += 1);
            assert_eq!(ran, 0);
            assert!(remove(&a.id));
            assert!(list().is_empty());
            assert!(add("", 1, Repeat::Once).is_err());
            assert!(add("x", 1, Repeat::Every(5)).is_err());
        });
    }
}
