//! The router of the text/voice commands of the new features (memory, translation, messenger). One entry: `handle(text, original)`;
//! `None` = not for these features, the normal command search goes on. Nothing here executes silently: the messenger always waits
//! for a confirmation, and every answer says what really happened.

use crate::{actionlog, autopilot, confirm, contacts, imagegen, kb, memory, messenger, notes, owner, perms, progsearch, scheduler, smarthome, store, translate};
use parking_lot::Mutex;

pub struct Reply {
    pub text: String,
    // the phrase to run as if the user had said it (the answer «да» to «Вы имели в виду …?»)
    pub rerun: Option<String>,
}

fn r(s: impl Into<String>) -> Option<Reply> {
    Some(Reply { text: s.into(), rerun: None })
}

// the message that waits for «да» / «нет» (and when it was prepared)
static PENDING_MSG: Mutex<Option<(String, u64)>> = parking_lot::const_mutex(None);
const PENDING_TTL: u64 = 120;

#[cfg(all(feature = "lua", target_os = "windows"))]
fn open(target: &str) -> Result<(), String> {
    crate::lua::shell_open(target)
}
#[cfg(not(all(feature = "lua", target_os = "windows")))]
fn open(_target: &str) -> Result<(), String> {
    Err("открытие ссылок доступно только в Windows".into())
}

pub fn pending_message() -> Option<String> {
    let p = PENDING_MSG.lock();
    p.as_ref().filter(|(_, t)| store::now() - t <= PENDING_TTL).map(|(id, _)| id.clone())
}

fn adapter_for(channel: &str) -> messenger::LinkAdapter {
    messenger::LinkAdapter { channel: channel.to_string() }
}

fn describe(o: &messenger::Outgoing) -> String {
    match o.status {
        messenger::Status::ManualSendRequired if o.channel == "telegram" => format!("Открыл чат с «{}» в Telegram, но БЕЗ текста: Telegram не принимает его из ссылки. Вставьте сообщение сами (оно в истории) и нажмите «Отправить»; я не узнаю, отправили ли вы.", o.contact),
        messenger::Status::ManualSendRequired => format!("Открыл чат с «{}» в WhatsApp с готовым текстом. Нажмите «Отправить» там — сам я отправить не могу и не узнаю, отправили ли вы.", o.contact),
        messenger::Status::Sent => format!("Отправлено: «{}».", o.contact),
        messenger::Status::Error => format!("Не отправлено: {}.", o.note),
        s => format!("Сообщение «{}»: {}.", o.contact, s.label()),
    }
}

// the same «да» as by voice, from the buttons of the window: the adapter is called once, a link is opened for the manual send
pub fn confirm_message(id: &str) -> Result<messenger::Outgoing, String> {
    let channel = messenger::get(id).map(|o| o.channel).ok_or("такого сообщения нет")?;
    let o = messenger::confirm(id, &adapter_for(&channel))?;
    if pending_message().as_deref() == Some(id) {
        *PENDING_MSG.lock() = None;
    }
    if o.status == messenger::Status::ManualSendRequired {
        open(&o.url).map_err(|e| format!("Не удалось открыть чат: {}", e))?;
    }
    Ok(o)
}

pub fn cancel_message(id: &str) -> Result<messenger::Outgoing, String> {
    if pending_message().as_deref() == Some(id) {
        *PENDING_MSG.lock() = None;
    }
    messenger::cancel(id)
}

// «Вы имели в виду «открой браузер»?»: the phrase waits for «да» (60 s)
static PENDING_CLARIFY: Mutex<Option<(String, u64)>> = parking_lot::const_mutex(None);
const CLARIFY_TTL: u64 = 60;

// the question for a phrase that was not understood exactly (commands::fuzzy_decide)
pub fn clarify(options: &[String]) -> String {
    match options {
        [one] => {
            *PENDING_CLARIFY.lock() = Some((one.clone(), store::now()));
            format!("Не расслышал точно. Вы имели в виду «{}»? Скажите «да» или «нет».", one)
        }
        [a, b, ..] => {
            *PENDING_CLARIFY.lock() = None;
            format!("Не расслышал точно: «{}» или «{}»? Назовите нужное.", a, b)
        }
        [] => String::new(),
    }
}

fn answer_clarify(text: &str) -> Option<Reply> {
    let pending = PENDING_CLARIFY.lock().clone().filter(|(_, t)| store::now().saturating_sub(*t) <= CLARIFY_TTL);
    let (phrase, _) = pending?;
    match confirm::classify(text) {
        confirm::Answer::Yes => {
            *PENDING_CLARIFY.lock() = None;
            Some(Reply { text: String::new(), rerun: Some(phrase) })
        }
        confirm::Answer::No => {
            *PENDING_CLARIFY.lock() = None;
            r("Хорошо, не выполняю.")
        }
        // anything else is a new phrase: the question is dropped
        confirm::Answer::Other => {
            *PENDING_CLARIFY.lock() = None;
            None
        }
    }
}

fn answer_pending(text: &str) -> Option<Reply> {
    let id = pending_message()?;
    match confirm::classify(text) {
        confirm::Answer::Yes => {
            *PENDING_MSG.lock() = None;
            match messenger::get(&id).map(|o| o.channel) {
                Some(ch) => match messenger::confirm(&id, &adapter_for(&ch)) {
                    Ok(o) => {
                        if o.status == messenger::Status::ManualSendRequired {
                            if let Err(e) = open(&o.url) {
                                return r(format!("Не удалось открыть чат: {}. Текст сообщения сохранён в истории.", e));
                            }
                        }
                        r(describe(&o))
                    }
                    Err(e) => r(e),
                },
                None => r("Сообщение не найдено."),
            }
        }
        confirm::Answer::No => {
            *PENDING_MSG.lock() = None;
            match messenger::cancel(&id) {
                Ok(_) => r("Отменено, ничего не отправлено."),
                Err(e) => r(e),
            }
        }
        confirm::Answer::Other => None,
    }
}

// a result that comes later (a picture is drawn for a minute): the app says it the way it says everything
static ANNOUNCER: once_cell::sync::OnceCell<Box<dyn Fn(&str) + Send + Sync>> = once_cell::sync::OnceCell::new();
pub fn set_announcer(f: Box<dyn Fn(&str) + Send + Sync>) {
    let _ = ANNOUNCER.set(f);
}
fn announce_later(text: &str) {
    match ANNOUNCER.get() {
        Some(f) => f(text),
        None => log::info!("{} chars", text.chars().count()),
    }
}

fn do_translate(req: &translate::Request) -> String {
    if let Err(e) = perms::check(perms::Cap::AiNetwork) {
        return e;
    }
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let ask = |system: &str, user: &str| crate::ai::complete(system, user, &cancel).map_err(|e| e.user_text());
    match translate::translate(req, &ask) {
        Ok(t) => t,
        Err(e) => e,
    }
}

fn do_messenger(p: &messenger::Parsed) -> Option<Reply> {
    if let Err(e) = perms::check(perms::Cap::Messenger).and_then(|_| owner::guard()) {
        return r(e);
    }
    match messenger::prepare(p) {
        messenger::Prepared::Ask(q) => r(q),
        messenger::Prepared::Ready(o) => {
            if let Err(e) = messenger::stage(&o) {
                return r(e);
            }
            autopilot::clear();
            *PENDING_MSG.lock() = Some((o.id.clone(), store::now()));
            r(format!("Сообщение для «{}» ({}): «{}». Отправить? Скажите «да» или «нет».", o.contact, if o.channel == "whatsapp" { "WhatsApp" } else { "Telegram" }, o.body))
        }
    }
}

fn do_memory(text: &str) -> Option<Reply> {
    if let Some(fact) = memory::parse_remember(text) {
        if let Err(e) = perms::check(perms::Cap::Memory) {
            return r(e);
        }
        return match memory::add(&fact, "command") {
            Ok(memory::Added::Added(id)) => {
                actionlog::record(&format!("Запомнил: {}", fact), actionlog::Undo::MemoryAdd(id));
                r(format!("Запомнил: {}.", fact))
            }
            Ok(memory::Added::Duplicate(_)) => r("Это я уже помню."),
            Ok(memory::Added::Conflict(_, old)) => {
                // the old fact is replaced only when the user says so
                r(format!("Раньше я помнил иначе: «{}». Если нужно заменить, скажите «забудь» и это старое, затем повторите.", old))
            }
            Err(e) => r(e),
        };
    }
    if let Some(q) = memory::parse_recall(text) {
        let hits = memory::search(&q);
        return if hits.is_empty() {
            r("Ничего об этом не помню.")
        } else {
            r(hits.iter().take(5).map(|e| e.text.clone()).collect::<Vec<_>>().join(". "))
        };
    }
    if let Some(q) = memory::parse_forget(text) {
        if let Err(e) = owner::guard() {
            return r(e);
        }
        let hits = memory::search(&q);
        return match hits.len() {
            0 => r("Такого в памяти нет."),
            1 => {
                if memory::delete(&hits[0].id) {
                    actionlog::record(&format!("Забыл: {}", hits[0].text), actionlog::Undo::MemoryRestore(hits[0].clone()));
                    r(format!("Забыл: {}. Если это ошибка, скажите «верни как было».", hits[0].text))
                } else {
                    r("Не получилось забыть: запись не удалилась.")
                }
            }
            n => r(format!("Подходит {} записей, уточните, какую забыть: {}.", n, hits.iter().take(3).map(|e| format!("«{}»", e.text)).collect::<Vec<_>>().join(", "))),
        };
    }
    None
}

fn local_now() -> (chrono::NaiveDateTime, i64) {
    let l = chrono::Local::now();
    (l.naive_local(), l.offset().local_minus_utc() as i64)
}

fn do_notes(text: &str) -> Option<Reply> {
    use notes::Cmd;
    let cmd = notes::parse(text)?;
    if let Err(e) = perms::check(perms::Cap::Memory) {
        return r(e);
    }
    let (now, off) = local_now();
    match cmd {
        Cmd::Write { text, kind } => match notes::add(&text, kind, store::now()) {
            Ok(n) => {
                actionlog::record(&format!("Записал в {}: {}", if kind == "diary" { "дневник" } else { "заметки" }, text), actionlog::Undo::NoteAdd(n.id));
                r(if kind == "diary" { "Записал в дневник." } else { "Записал заметку." })
            }
            Err(e) => r(e),
        },
        Cmd::Read { days_back } => {
            let list = notes::on_day(notes::day_for(now.date(), days_back), off);
            if list.is_empty() {
                r(format!("За {} записей нет.", if days_back == 0 { "сегодня" } else { "вчера" }))
            } else {
                r(list.iter().map(|n| n.text.clone()).collect::<Vec<_>>().join(". "))
            }
        }
        Cmd::Find(q) => {
            let h = notes::search(&q);
            if h.is_empty() { r("В дневнике этого нет.") } else { r(h.iter().take(5).map(|n| n.text.clone()).collect::<Vec<_>>().join(". ")) }
        }
    }
}

fn do_scheduler(text: &str) -> Option<Reply> {
    let (now, off) = local_now();
    let plan = scheduler::parse(text, now, off)?;
    if let Err(e) = perms::check(perms::Cap::Scheduler).and_then(|_| owner::guard()) {
        return r(e);
    }
    match scheduler::add(&plan.text, plan.at, plan.repeat) {
        Ok(j) => {
            actionlog::record(&format!("Задание: {}", plan.text), actionlog::Undo::JobAdd(j.id.clone()));
            r(format!("Поставил: {}.", scheduler::describe(&j, off)))
        }
        Err(e) => r(e),
    }
}

fn do_images(text: &str) -> Option<Reply> {
    let (video, prompt) = imagegen::parse(text)?;
    if video {
        return r(imagegen::generate_video(&prompt).unwrap_err());
    }
    if let Err(e) = perms::check(perms::Cap::AiNetwork) {
        return r(e);
    }
    std::thread::spawn(move || match imagegen::generate(&prompt) {
        Ok(p) => {
            actionlog::record(&format!("Нарисовал: {}", prompt), actionlog::Undo::FileCreated(p.to_string_lossy().into()));
            announce_later(&format!("Готово, картинка сохранена: {}", p.display()));
        }
        Err(e) => announce_later(&format!("Не получилось нарисовать: {}", e)),
    });
    r("Рисую, это может занять минуту. Скажу, когда будет готово.")
}

// «найди через Bixby …»: Samsung Bixby has no public search interface for a computer, so Jarvis says so and searches nowhere in its name
fn do_bixby(text: &str) -> Option<Reply> {
    let n = crate::textnorm::normalize(text);
    let mentions = n.split(' ').any(|w| matches!(w, "bixby" | "биксби" | "бикпсби" | "бикси" | "биксбии"));
    let asks = ["найди", "поищи", "ищи", "search", "find", "look", "izla", "top"].iter().any(|v| n.split(' ').any(|w| w == *v));
    if !(mentions && asks) {
        return None;
    }
    r("У Bixby нет открытого интерфейса поиска для компьютера, поэтому я не могу искать «через Bixby» и не буду делать вид, что искал им. Скажите «найди в интернете …», и я поищу обычным поиском, назвав, чем искал.")
}

fn do_programs(text: &str) -> Option<Reply> {
    let n = crate::textnorm::normalize(text);
    let q = ["найди программу ", "найди приложение ", "find program "].iter().find_map(|p| n.strip_prefix(p))?.to_string();
    Some(Reply {
        rerun: None,
        text: match progsearch::search(&q) {
            Ok(l) if l.is_empty() => "В каталоге Windows такой программы нет.".into(),
            Ok(l) => format!("Нашёл в каталоге Windows: {}. Чтобы поставить, откройте Программы → Каталог и нажмите «Установить»: молча я ничего не ставлю.", l.iter().take(3).map(|p| format!("{} ({})", p.name, p.version)).collect::<Vec<_>>().join(", ")),
            Err(e) => e,
        },
    })
}

fn answer_plan(text: &str) -> Option<Reply> {
    let run = |sc: crate::scenario::Scenario| {
        std::thread::spawn(move || {
            let r = autopilot::execute(&sc);
            actionlog::record(&format!("План выполнен: {}", sc.name), actionlog::Undo::None);
            announce_later(&r.message);
        });
    };
    match autopilot::answer(text, &run) {
        autopilot::Answer::NotMine => None,
        autopilot::Answer::Started => r("Выполняю по плану. Скажите «отмени план», чтобы остановить перед следующим шагом."),
        autopilot::Answer::Cancelled => r("Отменено, ничего не выполнено."),
    }
}

// `text` is the original phrase of the user (capitals and punctuation kept)
pub fn handle(text: &str) -> Option<Reply> {
    if let Some(a) = answer_clarify(text) {
        return Some(a);
    }
    if let Some(a) = answer_pending(text) {
        return Some(a);
    }
    if let Some(a) = answer_plan(text) {
        return Some(a);
    }
    if autopilot::stop_requested(text) {
        return r(if crate::scenario::cancel() { "Остановлю перед следующим шагом." } else if autopilot::has_pending() { autopilot::clear(); "План отменён." } else { "Сейчас ничего не выполняется." });
    }
    if crate::smsread::repeat_requested(text) {
        return r(crate::smsread::repeat_last());
    }
    if let Some(req) = autopilot::parse(text) {
        *PENDING_MSG.lock() = None;
        return r(match autopilot::propose(&req) { Ok(d) => d, Err(e) => e });
    }
    if let Some(undo) = actionlog::parse(text) {
        return r(if undo { match owner::guard().and_then(|_| actionlog::undo("")) { Ok(m) => m, Err(e) => e } } else { actionlog::summary(5) });
    }
    if let Some(req) = translate::parse(text) {
        return r(do_translate(&req));
    }
    if let Some(p) = messenger::parse(text) {
        return do_messenger(&p);
    }
    if let Some(r) = do_bixby(text) {
        return Some(r);
    }
    if let Some(r) = do_images(text) {
        return Some(r);
    }
    if let Some(a) = kb::handle(text) {
        return r(a);
    }
    if let Some(r) = do_notes(text) {
        return Some(r);
    }
    if let Some(r) = do_scheduler(text) {
        return Some(r);
    }
    if let Some(r) = do_programs(text) {
        return Some(r);
    }
    if let Some(m) = crate::wol::handle(text) {
        return r(m);
    }
    if let Some(m) = smarthome::handle(text) {
        return r(m);
    }
    let _ = &contacts::list;
    if let Some(r) = do_memory(text) {
        return Some(r);
    }
    // «запоминать самому»: a plain statement about the user (name, city, allergy…) is kept
    if memory::load().auto_remember && perms::allowed(perms::Cap::Memory) {
        if let Some(fact) = memory::auto_fact(text) {
            if let Ok(memory::Added::Added(id)) = memory::add(&fact, "auto") {
                actionlog::record(&format!("Запомнил сам: {}", fact), actionlog::Undo::MemoryAdd(id));
                return r(format!("Запомнил: {}. Если не нужно, скажите «верни как было».", fact));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_flow() {
        store::with_temp(|| {
            assert_eq!(handle("запомни, что мой кот — Барсик").unwrap().text, "Запомнил: мой кот — Барсик.");
            assert!(handle("что ты помнишь про кота").unwrap().text.contains("Барсик"));
            assert!(handle("запомни мой пароль 1234").unwrap().text.contains("не запоминаю"));
            assert!(handle("забудь про кота").unwrap().text.starts_with("Забыл"));
            assert_eq!(handle("что ты помнишь про кота").unwrap().text, "Ничего об этом не помню.");
            assert!(handle("открой браузер").is_none());
        });
    }
    #[test]
    fn a_clarifying_question_runs_the_offered_phrase_only_on_yes() {
        store::with_temp(|| {
            assert!(clarify(&["открой браузер".into()]).contains("Вы имели в виду «открой браузер»"));
            let r = handle("да").unwrap();
            assert_eq!(r.rerun.as_deref(), Some("открой браузер"));
            // answered once
            assert!(handle("да").is_none());
            clarify(&["закрой браузер".into()]);
            assert_eq!(handle("нет").unwrap().text, "Хорошо, не выполняю.");
            assert!(handle("да").is_none());
            // another phrase drops the question
            clarify(&["закрой браузер".into()]);
            assert!(handle("что такое питон").is_none());
            assert!(handle("да").is_none());
            // two candidates: a question without a pending «да»
            let q = clarify(&["открой браузер".into(), "закрой браузер".into()]);
            assert!(q.contains("или"));
            assert!(handle("да").is_none());
        });
    }

    #[test]
    fn bixby_is_not_pretended() {
        let r = do_bixby("найди через Bixby рецепт борща").unwrap().text;
        assert!(r.contains("нет открытого интерфейса") && r.contains("не буду делать вид"));
        assert!(do_bixby("найди рецепт борща").is_none());
        assert!(do_bixby("привет Bixby").is_none());
    }

    #[test]
    fn messenger_waits_for_the_answer() {
        store::with_temp(|| {
            contacts::save(contacts::Contact { name: "Малика".into(), aliases: vec!["жена".into()], phone: "998901234567".into(), ..Default::default() }).unwrap();
            let q = handle("напиши жене: буду поздно").unwrap().text;
            assert!(q.contains("Отправить?"), "{}", q);
            // a different phrase does not answer it
            assert!(handle("открой браузер").is_none());
            let id = pending_message().unwrap();
            assert_eq!(messenger::get(&id).unwrap().status, messenger::Status::AwaitingConfirmation);
            assert_eq!(handle("нет").unwrap().text, "Отменено, ничего не отправлено.");
            assert_eq!(messenger::get(&id).unwrap().status, messenger::Status::Cancelled);
            assert!(pending_message().is_none());
            // unknown person: a question, nothing staged
            assert!(handle("напиши начальнику: привет").unwrap().text.contains("Не нашёл"));
            assert!(pending_message().is_none());
        });
    }
}
