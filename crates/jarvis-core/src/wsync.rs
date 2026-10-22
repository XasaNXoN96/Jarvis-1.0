//! Workspace sync WITHOUT a server of ours: the memory, the contacts and the notes of two (or more) computers are merged through ONE FILE in a
//! folder that the user already syncs by something else (OneDrive, Dropbox, Syncthing, a USB stick). Every sync reads that file, merges it with
//! the local data and writes the merged result back. The newer version of an item wins, a deleted item is carried as a tombstone for 30 days, and
//! what is merged is exactly what the report says. Settings, keys, tokens, messages and the schedule are NEVER put in the file. The file holds personal data
//! (contacts, notes), so the folder should be the user's own.

use crate::{contacts, memory, notes, store};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const STATE: &str = "sync_state.json";
pub const FILE_NAME: &str = "jarvis-workspace.json";
const TOMB_DAYS: u64 = 30;
pub const KINDS: [&str; 3] = ["memory", "contacts", "notes"];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Tomb {
    pub kind: String,
    pub id: String,
    pub at: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Bundle {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub written: u64,
    // kind → items (each one a JSON object with "id" and "updated")
    #[serde(default)]
    pub items: BTreeMap<String, Vec<Value>>,
    #[serde(default)]
    pub tombstones: Vec<Tomb>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct State {
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub last_sync: u64,
    // the ids that were present after the last sync (what disappears from here is a deletion made on this computer)
    #[serde(default)]
    pub last_ids: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub tombs: Vec<Tomb>,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub added: usize,
    pub updated: usize,
    pub deleted: usize,
    pub conflicts: usize,
    pub items_total: usize,
}

fn id_of(v: &Value) -> Option<String> {
    v.get("id").and_then(|i| i.as_str()).map(|s| s.to_string())
}
fn upd_of(v: &Value) -> u64 {
    v.get("updated").and_then(|u| u.as_u64()).unwrap_or(0)
}

// The merge itself (pure): local items, the file, the state → the new local items, the file to write, the report.
pub fn merge(local: &BTreeMap<String, Vec<Value>>, remote: &Bundle, state: &mut State, now: u64) -> (BTreeMap<String, Vec<Value>>, Bundle, Report) {
    let mut report = Report::default();
    // 1. what vanished here since the last sync was deleted here
    for k in KINDS {
        let now_ids: BTreeSet<String> = local.get(k).map(|v| v.iter().filter_map(id_of).collect()).unwrap_or_default();
        for id in state.last_ids.get(k).cloned().unwrap_or_default() {
            if !now_ids.contains(&id) && !state.tombs.iter().any(|t| t.kind == k && t.id == id) {
                state.tombs.push(Tomb { kind: k.into(), id, at: now });
            }
        }
    }
    // 2. all the tombstones, the newest per item
    let mut tombs: BTreeMap<(String, String), u64> = BTreeMap::new();
    for t in state.tombs.iter().chain(remote.tombstones.iter()) {
        let e = tombs.entry((t.kind.clone(), t.id.clone())).or_insert(0);
        *e = (*e).max(t.at);
    }
    // old tombstones are forgotten
    let horizon = now.saturating_sub(TOMB_DAYS * 86400);
    tombs.retain(|_, at| *at >= horizon);
    // 3. the merge per kind
    let mut out: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for k in KINDS {
        let l: BTreeMap<String, Value> = local.get(k).map(|v| v.iter().filter_map(|x| id_of(x).map(|i| (i, x.clone()))).collect()).unwrap_or_default();
        let r: BTreeMap<String, Value> = remote.items.get(k).map(|v| v.iter().filter_map(|x| id_of(x).map(|i| (i, x.clone()))).collect()).unwrap_or_default();
        let ids: BTreeSet<&String> = l.keys().chain(r.keys()).collect();
        let mut merged = Vec::new();
        for id in ids {
            let (li, ri) = (l.get(id), r.get(id));
            let newest = match (li, ri) {
                (Some(a), Some(b)) => {
                    if a == b {
                        a
                    } else if upd_of(b) > upd_of(a) {
                        // both changed since the last sync: a conflict (the newer wins, and the report says so)
                        if upd_of(a) > state.last_sync && upd_of(b) > state.last_sync {
                            report.conflicts += 1;
                        }
                        report.updated += 1;
                        b
                    } else {
                        if upd_of(a) > state.last_sync && upd_of(b) > state.last_sync && upd_of(a) != upd_of(b) {
                            report.conflicts += 1;
                        }
                        a
                    }
                }
                (Some(a), None) => a,
                (None, Some(b)) => b,
                (None, None) => continue,
            };
            if let Some(at) = tombs.get(&(k.to_string(), id.clone())) {
                if *at >= upd_of(newest) {
                    if li.is_some() {
                        report.deleted += 1;
                    }
                    continue;
                }
            }
            if li.is_none() {
                report.added += 1;
            }
            merged.push(newest.clone());
        }
        out.insert(k.to_string(), merged);
    }
    report.items_total = out.values().map(|v| v.len()).sum();
    // 4. the new state and the file
    state.last_ids = out.iter().map(|(k, v)| (k.clone(), v.iter().filter_map(id_of).collect())).collect();
    state.tombs = tombs.iter().map(|((k, id), at)| Tomb { kind: k.clone(), id: id.clone(), at: *at }).collect();
    state.last_sync = now;
    let bundle = Bundle { version: 1, device: state.device.clone(), written: now, items: out.clone(), tombstones: state.tombs.clone() };
    (out, bundle, report)
}

fn state() -> State {
    let mut s = store::read::<State>(STATE).0;
    if s.device.is_empty() {
        s.device = store::new_id();
    }
    s
}
fn save_state(s: &State) -> Result<(), String> {
    store::write(STATE, s)
}

pub fn folder() -> Option<String> {
    Some(state().folder).filter(|f| !f.is_empty())
}

pub fn set_folder(path: &str) -> Result<(), String> {
    crate::owner::guard()?;
    let p = Path::new(path.trim());
    if !p.is_dir() {
        return Err("Такой папки нет.".into());
    }
    if p.parent().is_none() {
        return Err("Выберите обычную папку, а не диск целиком.".into());
    }
    let mut s = state();
    s.folder = p.to_string_lossy().to_string();
    save_state(&s)
}

pub fn clear_folder() -> bool {
    let mut s = state();
    s.folder.clear();
    save_state(&s).is_ok()
}

fn to_values<T: Serialize>(v: &[T]) -> Vec<Value> {
    v.iter().filter_map(|x| serde_json::to_value(x).ok()).collect()
}

fn local_items() -> BTreeMap<String, Vec<Value>> {
    let mut m = BTreeMap::new();
    m.insert("memory".to_string(), to_values(&memory::load().items));
    m.insert("contacts".to_string(), to_values(&contacts::list()));
    m.insert("notes".to_string(), to_values(&notes::list()));
    m
}

fn from_values<T: serde::de::DeserializeOwned>(v: &[Value]) -> Vec<T> {
    v.iter().filter_map(|x| serde_json::from_value(x.clone()).ok()).collect()
}

pub fn file_path(folder: &str) -> PathBuf {
    Path::new(folder).join(FILE_NAME)
}

// reads the file of the folder; a file that cannot be read is NEVER overwritten
fn read_remote(path: &Path) -> Result<Bundle, String> {
    match std::fs::read_to_string(path) {
        Ok(t) => serde_json::from_str::<Bundle>(t.trim_start_matches('\u{feff}')).map_err(|_| format!("Файл {} не читается (повреждён или это не файл Jarvis). Я его не трогаю: уберите или почините его.", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Bundle::default()),
        Err(e) => Err(format!("Не удалось прочитать {}: {}", path.display(), e)),
    }
}

pub fn sync_now() -> Result<Report, String> {
    crate::owner::guard()?;
    let mut st = state();
    if st.folder.is_empty() {
        return Err("Папка синхронизации не выбрана.".into());
    }
    let path = file_path(&st.folder);
    let remote = read_remote(&path)?;
    let now = store::now();
    let (merged, bundle, report) = merge(&local_items(), &remote, &mut st, now);
    // the file first (if it cannot be written nothing changes here), then the local data
    let text = serde_json::to_string_pretty(&bundle).map_err(|e| e.to_string())?;
    crate::write_atomic(&path, text).map_err(|e| format!("Не удалось записать {}: {}", path.display(), e))?;
    let mem = memory::load();
    memory::replace_items(from_values(merged.get("memory").map(|v| v.as_slice()).unwrap_or(&[])), mem.auto_remember)?;
    contacts::replace_all(from_values(merged.get("contacts").map(|v| v.as_slice()).unwrap_or(&[])))?;
    notes::replace_all(from_values(merged.get("notes").map(|v| v.as_slice()).unwrap_or(&[])))?;
    save_state(&st)?;
    Ok(report)
}

pub fn status_json() -> Value {
    let s = state();
    serde_json::json!({ "folder": s.folder, "last_sync": s.last_sync, "file": if s.folder.is_empty() { Value::Null } else { Value::String(file_path(&s.folder).to_string_lossy().to_string()) } })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn items(kind: &str, v: Vec<Value>) -> BTreeMap<String, Vec<Value>> {
        let mut m = BTreeMap::new();
        m.insert(kind.to_string(), v);
        m
    }
    fn bundle(kind: &str, v: Vec<Value>, tombs: Vec<Tomb>) -> Bundle {
        Bundle { version: 1, device: "other".into(), written: 0, items: items(kind, v), tombstones: tombs }
    }
    fn ids(m: &BTreeMap<String, Vec<Value>>, k: &str) -> Vec<String> {
        m.get(k).map(|v| v.iter().filter_map(id_of).collect()).unwrap_or_default()
    }

    #[test]
    fn two_computers_get_each_others_items() {
        let mut st = State::default();
        let local = items("memory", vec![json!({"id":"a","text":"моё","updated":10})]);
        let remote = bundle("memory", vec![json!({"id":"b","text":"чужое","updated":20})], vec![]);
        let (out, file, rep) = merge(&local, &remote, &mut st, 100);
        assert_eq!(ids(&out, "memory"), vec!["a", "b"]);
        assert_eq!(ids(&file.items, "memory"), vec!["a", "b"]);
        assert_eq!((rep.added, rep.updated, rep.deleted, rep.conflicts, rep.items_total), (1, 0, 0, 0, 2));
        // the second sync changes nothing
        let (_, _, rep2) = merge(&out, &file, &mut st, 101);
        assert_eq!((rep2.added, rep2.updated, rep2.deleted), (0, 0, 0));
    }

    #[test]
    fn the_newer_version_wins_and_a_double_edit_is_a_conflict() {
        let mut st = State { last_sync: 50, ..Default::default() };
        let local = items("notes", vec![json!({"id":"n","text":"локально","updated":60})]);
        let remote = bundle("notes", vec![json!({"id":"n","text":"с другого","updated":70})], vec![]);
        let (out, _, rep) = merge(&local, &remote, &mut st, 100);
        assert_eq!(out["notes"][0]["text"], "с другого");
        assert_eq!((rep.updated, rep.conflicts), (1, 1));
        // an old remote copy does not overwrite a newer local one, and that is not a conflict
        let mut st = State { last_sync: 90, ..Default::default() };
        let local = items("notes", vec![json!({"id":"n","text":"новое","updated":95})]);
        let remote = bundle("notes", vec![json!({"id":"n","text":"старое","updated":40})], vec![]);
        let (out, _, rep) = merge(&local, &remote, &mut st, 100);
        assert_eq!(out["notes"][0]["text"], "новое");
        assert_eq!(rep.conflicts, 0);
    }

    #[test]
    fn a_deletion_travels_and_a_later_edit_beats_it() {
        // computer B deleted the item at 80: A had it (updated 50) → deleted on A
        let mut st = State::default();
        let local = items("memory", vec![json!({"id":"x","text":"t","updated":50})]);
        let remote = bundle("memory", vec![], vec![Tomb { kind: "memory".into(), id: "x".into(), at: 80 }]);
        let (out, file, rep) = merge(&local, &remote, &mut st, 100);
        assert!(ids(&out, "memory").is_empty());
        assert_eq!(rep.deleted, 1);
        assert!(file.tombstones.iter().any(|t| t.id == "x"), "the tombstone stays in the file for the others");
        // A edited the item AFTER B deleted it (95 > 80): the item survives
        let mut st = State::default();
        let local = items("memory", vec![json!({"id":"x","text":"снова","updated":95})]);
        let (out, _, rep) = merge(&local, &remote, &mut st, 100);
        assert_eq!(ids(&out, "memory"), vec!["x"]);
        assert_eq!(rep.deleted, 0);
    }

    #[test]
    fn what_vanished_here_becomes_a_tombstone_and_old_ones_are_forgotten() {
        let mut st = State::default();
        st.last_ids.insert("contacts".into(), vec!["c1".into(), "c2".into()]);
        let local = items("contacts", vec![json!({"id":"c1","name":"А","updated":1})]);
        let remote = bundle("contacts", vec![json!({"id":"c2","name":"Б","updated":1})], vec![]);
        let (out, file, _) = merge(&local, &remote, &mut st, 1000);
        assert_eq!(ids(&out, "contacts"), vec!["c1"], "c2 was deleted here, the file's copy does not bring it back");
        assert!(file.tombstones.iter().any(|t| t.id == "c2" && t.at == 1000));
        let mut st = State { tombs: vec![Tomb { kind: "memory".into(), id: "old".into(), at: 10 }], ..Default::default() };
        let (_, file, _) = merge(&BTreeMap::new(), &Bundle::default(), &mut st, 10 + TOMB_DAYS * 86400 + 5);
        assert!(file.tombstones.is_empty());
    }

    #[test]
    fn the_whole_cycle_through_a_folder_with_real_stores() {
        store::with_temp(|| {
            crate::owner::reset_for_tests();
            let dir = tempfile::tempdir().unwrap();
            set_folder(&dir.path().to_string_lossy()).unwrap();
            assert!(set_folder("/no/such/dir").is_err());
            memory::add("мой кот — Барсик", "t").unwrap();
            notes::add("заметка с этого компьютера", "note", 100).unwrap();
            let r = sync_now().unwrap();
            assert_eq!(r.items_total, 2);
            let text = std::fs::read_to_string(file_path(&dir.path().to_string_lossy())).unwrap();
            assert!(text.contains("Барсик") && !text.contains("token") && !text.contains("api_key"));
            // another computer wrote an item into the file
            let mut b: Bundle = serde_json::from_str(&text).unwrap();
            b.items.get_mut("notes").unwrap().push(json!({"id":"zz","text":"с другого компьютера","at":5,"kind":"note","updated":store::now()+1}));
            std::fs::write(file_path(&dir.path().to_string_lossy()), serde_json::to_string(&b).unwrap()).unwrap();
            let r = sync_now().unwrap();
            assert_eq!(r.added, 1);
            assert!(notes::list().iter().any(|n| n.text == "с другого компьютера"));
            // a deletion here goes to the file
            let id = notes::list().iter().find(|n| n.text.starts_with("заметка")).unwrap().id.clone();
            assert!(notes::delete(&id));
            sync_now().unwrap();
            let after: Bundle = serde_json::from_str(&std::fs::read_to_string(file_path(&dir.path().to_string_lossy())).unwrap()).unwrap();
            assert!(after.tombstones.iter().any(|t| t.id == id));
            assert!(!after.items["notes"].iter().any(|n| id_of(n).as_deref() == Some(&id)));
        });
    }

    #[test]
    fn a_broken_file_is_never_overwritten() {
        store::with_temp(|| {
            crate::owner::reset_for_tests();
            let dir = tempfile::tempdir().unwrap();
            set_folder(&dir.path().to_string_lossy()).unwrap();
            let f = file_path(&dir.path().to_string_lossy());
            std::fs::write(&f, "{not json").unwrap();
            assert!(sync_now().unwrap_err().contains("не читается"));
            assert_eq!(std::fs::read_to_string(&f).unwrap(), "{not json");
            clear_folder();
            assert!(sync_now().unwrap_err().contains("не выбрана"));
        });
    }
}
