use super::*;
use chrono::TimeZone;

fn now() -> chrono::DateTime<chrono::Local> {
    chrono::Local.with_ymd_and_hms(2026, 10, 11, 12, 0, 0).unwrap()
}

fn p(text: &str) -> Query {
    parse(text, now())
}

#[test]
fn the_phrase_becomes_a_query_name_kind_place_time_and_text() {
    assert_eq!(p("найди файл report").name, "report");
    let q = p("Найди документ с названием договор");
    assert_eq!((q.name.as_str(), q.kind.as_str()), ("договор", "документы"));
    assert!(q.exts.contains(&"docx".to_string()) && q.exts.contains(&"pdf".to_string()));
    let q = p("найди презентацию по информатике");
    assert_eq!(q.name, "информатике");
    assert!(q.exts.contains(&"pptx".to_string()));
    let q = p("найди все PDF-файлы");
    assert_eq!((q.name.as_str(), q.exts.clone()), ("", vec!["pdf".to_string()]));
    let q = p("найди файл с расширением .py");
    assert_eq!((q.name.as_str(), q.exts.clone()), ("", vec!["py".to_string()]));
    assert_eq!(p("найди все файлы docx").exts, ["docx"]);
    let q = p("найди папку ACCA USA");
    assert!(q.folders_only);
    assert_eq!(q.name, "acca usa");
    assert_eq!(p("найди файл, в названии которого есть слово экзамен").name, "экзамен");
    let q = p("Найди файл, содержащий текст ICTERA");
    assert_eq!((q.name.as_str(), q.content.as_str()), ("", "ICTERA"));
    let q = p("найди документ отчёт с текстом «Итоги года»");
    assert_eq!((q.name.as_str(), q.content.as_str()), ("отчет", "Итоги года"));

    let q = p("найди фотографии за октябрь");
    assert!(q.exts.contains(&"jpg".to_string()));
    let (a, b) = month_range(2026, 10);
    assert_eq!((q.since, q.until), (a, b));
    let q = p("найди фото за декабрь");
    assert_eq!(q.since, month_range(2025, 12).0, "a month still ahead is the one of last year");
    assert_eq!(p("найди файлы за май 2024").since, month_range(2024, 5).0);
    let q = p("найди недавно изменённые документы");
    assert!(q.recent && q.since.is_some() && q.name.is_empty());
    let q = p("найди файл, который я недавно скачал");
    assert_eq!(q.scope, Scope::Downloads);
    assert!(q.recent);
    assert_eq!(p("найди файл отчёт на рабочем столе").scope, Scope::Desktop);
    let q = p("найди файл на рабочем столе");
    assert_eq!((q.scope.clone(), q.name.as_str()), (Scope::Desktop, ""));
    assert_eq!(p("найди файл на диске D").scope, Scope::Drive('D'));
    assert_eq!(p("найди файл бюджет на диске д").scope, Scope::Drive('D'));
    assert_eq!(p("найди отчёт в документах").scope, Scope::Documents);
    assert_eq!(p("найди отчёт в загрузках").scope, Scope::Downloads);
    assert_eq!(p("найди отчёт везде").scope, Scope::Everywhere);
    assert_eq!(p("найди файл один два три").name, "123");
    assert_eq!(p("найди файл Мой Отчёт 2024 Final").name, "отчет 2024 final", "mixed case and spaces");
    assert!(is_empty_query(&p("найди файл")));
    assert_eq!(p("find all pdf files").exts, ["pdf"]);
    // names that only begin like a month or a kind are names
    let q = p("найди файл мартин");
    assert_eq!((q.name.as_str(), q.since), ("мартин", None));
    assert_eq!(p("найди файл августина").since, None);
    let q = p("найди файл фотошоп");
    assert_eq!((q.name.as_str(), q.exts.len()), ("фотошоп", 0));
    assert!(p("найди фотографии").exts.contains(&"jpg".to_string()));
    assert_eq!(p("найди файлы с марта").since, month_range(2026, 3).0, "the forms of a month name");
    assert_eq!(p("найди файлы за март").since, month_range(2026, 3).0);
    // a folder named like a kind of file
    let q = p("найди папку фото");
    assert!(q.folders_only && q.exts.is_empty());
    assert_eq!(q.name, "фото");
    for (phrase, ext) in [("найди аудиофайлы", "mp3"), ("найди видеозаписи", "mp4"), ("найди музыкальные файлы", "mp3"), ("найди файлы с фотографиями", "jpg"), ("найди видео", "mp4")] {
        assert!(p(phrase).exts.contains(&ext.to_string()), "{}", phrase);
    }
    assert!(p("найди файл видеокарта").exts.is_empty(), "a word that only begins like «видео» is a name");
    // the text marker as whole words
    assert_eq!(p("найди файл в котором содержится слово ключ").content, "ключ");
    // "в папке видео" is the folder
    let q = p("найди отчёт в папке видео");
    assert_eq!((q.scope.clone(), q.name.as_str(), q.exts.len()), (Scope::Videos, "отчет", 0));
    assert_eq!(p("find file report on drive e").scope, Scope::Drive('E'));
}

fn make_zip(entries: &[(&str, &str)]) -> Vec<u8> {
    use std::io::Write;
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, text) in entries {
        let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(text.as_bytes()).unwrap();
        let data = enc.finish().unwrap();
        let offset = out.len() as u32;
        out.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04, 20, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(text.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&data);
        central.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02, 20, 0, 20, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(text.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0]);
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out
}

#[cfg(not(target_os = "windows"))]
struct Home {
    dir: PathBuf,
    old: Option<String>,
}
#[cfg(not(target_os = "windows"))]
impl Drop for Home {
    fn drop(&mut self) {
        match &self.old { Some(v) => std::env::set_var("HOME", v), None => std::env::remove_var("HOME") }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[cfg(not(target_os = "windows"))]
// a home folder with Desktop / Documents / Downloads and files of many kinds (not on Windows: there the home is real)
fn home_with_files() -> Home {
    let dir = std::env::temp_dir().join(format!("jarvis_files_{}_{}", std::process::id(), rand::random::<u32>()));
    for d in ["Desktop", "Documents/Учёба/Информатика", "Downloads", "Pictures", "Documents/ACCA USA", "Documents/locked"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    let w = |rel: &str, data: &[u8]| std::fs::write(dir.join(rel), data).unwrap();
    w("Desktop/Report 2024.pdf", b"%PDF ICTERA");
    w("Desktop/notes.txt", "встреча с ICTERA в среду".as_bytes());
    w("Documents/Договор аренды.docx", &make_zip(&[("word/document.xml", "<w:document><w:t>Договор</w:t><w:t>ICT</w:t><w:t>ERA</w:t></w:document>")]));
    w("Documents/Учёба/Информатика/Презентация Информатика.pptx", &make_zip(&[("ppt/slides/slide1.xml", "<p:sld><a:t>Алгоритмы</a:t></p:sld>")]));
    w("Documents/Учёба/экзамен вопросы.docx", &make_zip(&[("word/document.xml", "<w:t>вопросы</w:t>")]));
    w("Documents/script.py", b"print('hi')");
    w("Documents/MiXeD CaSe File.TXT", b"x");
    w("Downloads/setup.exe", b"MZ");
    w("Downloads/report-final.PDF", b"%PDF");
    w("Pictures/IMG_0001.jpg", b"jpg");
    let old = std::env::var("HOME").ok();
    std::env::set_var("HOME", &dir);
    Home { dir, old }
}

#[cfg(not(target_os = "windows"))]
fn names(q: &Query) -> Vec<String> {
    run_for_tests(q).unwrap().iter().map(|i| i.path.file_name().unwrap().to_string_lossy().to_string()).collect()
}

#[cfg(not(target_os = "windows"))]
#[test]
fn files_are_found_by_name_part_extension_kind_place_and_text() {
    let _s = crate::session::TEST_LOCK.lock();
    let _g = TEST_LOCK.lock();
    let h = home_with_files();
    assert_eq!(names(&p("найди файл Report 2024"))[0], "Report 2024.pdf", "the exact name first");
    let r = names(&p("найди файл report"));
    assert!(r.contains(&"Report 2024.pdf".to_string()) && r.contains(&"report-final.PDF".to_string()), "a part of the name, any case: {:?}", r);
    let r = names(&p("найди все PDF-файлы"));
    assert_eq!(r.len(), 2, "{:?}", r);
    assert_eq!(names(&p("найди файл с расширением .py")), ["script.py"]);
    assert_eq!(names(&p("найди файл mixed case file")), ["MiXeD CaSe File.TXT"]);
    assert_eq!(names(&p("найди презентацию по информатике")), ["Презентация Информатика.pptx"]);
    assert_eq!(names(&p("найди документ с названием договор")), ["Договор аренды.docx"]);
    assert_eq!(names(&p("найди файл, в названии которого есть слово экзамен")), ["экзамен вопросы.docx"]);
    assert_eq!(names(&p("найди папку ACCA USA")), ["ACCA USA"], "a folder, not files");
    assert_eq!(names(&p("найди файл на рабочем столе")).len(), 2);
    assert_eq!(names(&p("найди файл report в загрузках")), ["report-final.PDF"]);
    assert_eq!(names(&p("найди фотографии")), ["IMG_0001.jpg"]);
    let r = names(&p("найди файл, содержащий текст ICTERA"));
    assert!(r.contains(&"notes.txt".to_string()) && r.contains(&"Договор аренды.docx".to_string()), "text files and Word documents: {:?}", r);
    assert!(!r.contains(&"Report 2024.pdf".to_string()), "a PDF is not read and never claimed to contain the text");
    assert!(names(&p("найди файл абракадабра")).is_empty());
    // the files of today (all were just written) and none of last year
    assert_eq!(names(&p("найди файлы за сегодня")).len(), names(&Query { name: String::new(), since: Some(0), ..Query::default() }).len());
    assert!(names(&Query { exts: vec!["pdf".into()], since: month_range(2020, 1).0, until: month_range(2020, 2).0, ..Query::default() }).is_empty());
    // a drive that does not exist, a missing folder
    assert!(roots(&Scope::Drive('Q')).unwrap_err().contains("Диска Q"));
    std::fs::remove_dir_all(h.dir.join("Pictures")).unwrap();
    assert!(roots(&Scope::Pictures).is_err(), "a missing folder is reported");
    drop(h);
}

#[cfg(all(unix, not(target_os = "windows")))]
#[test]
fn a_folder_without_access_is_skipped_and_counted() {
    use std::os::unix::fs::PermissionsExt;
    let _s = crate::session::TEST_LOCK.lock();
    let _g = TEST_LOCK.lock();
    let h = home_with_files();
    let locked = h.dir.join("Documents/locked");
    std::fs::write(locked.join("secret report.txt"), b"x").unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let readable = std::fs::read_dir(&locked).is_ok(); // root ignores permissions
    let (found, s) = run(&p("найди файл secret"), GENERATION.load(Ordering::SeqCst)).unwrap();
    if !readable {
        assert!(found.is_empty());
        assert!(s.denied >= 1);
    }
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    drop(h);
}

#[test]
fn office_documents_are_read_and_unsupported_formats_are_not() {
    let dir = std::env::temp_dir().join(format!("jarvis_zip_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let docx = dir.join("a.docx");
    std::fs::write(&docx, make_zip(&[("word/document.xml", "<w:t>Привет &amp; пока</w:t>"), ("word/styles.xml", "<x>НЕ ТО</x>")])).unwrap();
    let t = file_text(&docx, "docx").unwrap();
    assert!(t.contains("Привет & пока") && !t.contains("НЕ ТО"), "{}", t);
    let xlsx = dir.join("b.xlsx");
    std::fs::write(&xlsx, make_zip(&[("xl/sharedStrings.xml", "<si><t>Бюджет</t></si>")])).unwrap();
    assert!(file_text(&xlsx, "xlsx").unwrap().contains("Бюджет"));
    let utf16 = dir.join("c.txt");
    let mut bytes = vec![0xFF, 0xFE];
    for u in "текст".encode_utf16() { bytes.extend_from_slice(&u.to_le_bytes()); }
    std::fs::write(&utf16, bytes).unwrap();
    assert_eq!(file_text(&utf16, "txt").unwrap(), "текст");
    let broken = dir.join("d.docx");
    std::fs::write(&broken, b"not a zip").unwrap();
    assert_eq!(file_text(&broken, "docx").unwrap(), "", "a damaged document gives no text, no crash");
    assert!(file_text(&dir.join("e.pdf"), "pdf").is_none());
    let ansi = dir.join("f.txt");
    std::fs::write(&ansi, [0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2, 0x20, 0xA8, 0xEB, 0xEA, 0xE0]).unwrap(); // "Привет Ёлка" in Windows-1251
    assert_eq!(file_text(&ansi, "txt").unwrap(), "Привет Ёлка", "old Notepad files are read too");
    assert!(!content_supported("pdf") && !content_supported("doc") && content_supported("docx") && content_supported("py"));
    assert!(text_contains(&strip_xml("<w:t>ICT</w:t><w:t>ERA</w:t>"), "ictera"), "a word split into runs is still found");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_list_shows_name_type_size_date_and_place_and_continues_with_more() {
    let _g = TEST_LOCK.lock();
    assert_eq!(human_size(512), "512 Б");
    assert_eq!(human_size(2048), "2,0 КБ");
    assert_eq!(human_size(5 * 1024 * 1024 + 1024 * 300), "5,3 МБ");
    let line = item_line(0, &FoundItem { path: PathBuf::from("/x/Отчёт.PDF"), is_dir: false, size: 2048, modified: 0, score: 0 });
    assert!(line.starts_with("1. Отчёт.PDF — PDF, 2,0 КБ") && line.contains("(/x)"), "{}", line);
    let many: Vec<PathBuf> = (1..=11).map(|i| PathBuf::from(format!("/tmp/f{}.txt", i))).collect();
    set_results_for_tests(many);
    let m = more();
    assert!(m.starts_with("Результаты 9–11 из 11") && m.contains("11. f11.txt"), "{}", m);
    assert!(more().contains("Больше результатов нет"));
    assert_eq!(result_path(2), Some(PathBuf::from("/tmp/f2.txt")));
    assert_eq!(result_path(0), None);
    assert_eq!(result_path(99), None);
    set_results_for_tests(Vec::new());
}

#[test]
fn the_form_of_the_window_makes_the_same_query() {
    let q = query_from_form("Отчёт", ".pdf", "downloads", "", "");
    assert_eq!((q.name.as_str(), q.exts.clone(), q.scope.clone()), ("отчёт", vec!["pdf".to_string()], Scope::Downloads));
    let q = query_from_form("", "фото", "d", "за октябрь", "");
    assert_eq!(q.scope, Scope::Drive('D'));
    assert!(q.exts.contains(&"jpg".to_string()) && q.since.is_some());
    assert!(query_from_form("", "folder", "", "", "").folders_only);
}

#[test]
fn folders_of_users_are_not_skipped_on_windows_and_system_ones_are() {
    assert!(skip_dir("windows") && skip_dir("appdata") && skip_dir("node_modules"));
    assert_eq!(skip_dir("dev"), cfg!(not(target_os = "windows")), "\\Documents\\dev is an ordinary folder on Windows");
}

#[test]
fn text_of_an_unknown_encoding_is_decoded_sensibly() {
    let mut utf8 = "Привет, это документ про экзамен".as_bytes().to_vec();
    utf8.push(0xFF); // one broken byte
    assert!(decode_text(utf8).contains("документ про экзамен"), "a UTF-8 file with one bad byte stays UTF-8");
    assert_eq!(decode_text(vec![0xEC, 0xEE, 0xEB, 0xEE, 0xEA, 0xEE]), "молоко", "Windows-1251");
    assert_eq!(decode_text(b"plain".to_vec()), "plain");
}

