// Diagnostics: the real state of every part of Jarvis, with one of four statuses:
//   ok ("Работает"), warn ("Предупреждение"), error ("Ошибка"), unchecked ("Не проверено").
//
// Rules: a file that exists does not prove that a function works, a key does not prove that the AI is connected,
// a Piper process does not prove that speech can be made, a local server does not prove that a phone can reach it.
// So a check either proves something (a real frame from the microphone, a real sound file made by Piper, a real
// answer of the AI, a real request from a phone) or says "not checked" with the reason.
// The diagnostics never send a request to the AI, never change a setting and never run a command that changes the
// computer. The report is built from the last results only (copying it runs nothing) and contains no keys, tokens,
// conversations or texts of notifications.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Error,
    Unchecked,
}

impl Status {
    pub fn text(&self) -> &'static str {
        match self {
            Status::Ok => "Работает",
            Status::Warn => "Предупреждение",
            Status::Error => "Ошибка",
            Status::Unchecked => "Не проверено",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Check {
    pub key: &'static str,
    pub name: &'static str,
    pub status: Status,
    pub detail: String,
    pub fix: String,
    // a fix the owner can confirm in the window (from the fixed list below); "" = the advice is text only
    pub fix_id: &'static str,
}

fn check(key: &'static str, name: &'static str, status: Status, detail: impl Into<String>, fix: impl Into<String>) -> Check {
    Check { key, name, status, detail: detail.into(), fix: fix.into(), fix_id: "" }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn ago(t: u64) -> String {
    let s = now().saturating_sub(t);
    if s < 60 { format!("{} с назад", s) } else if s < 3600 { format!("{} мин назад", s / 60) } else { format!("{} ч назад", s / 3600) }
}

// ---------------------------------------------------------------------------------------------
// the checks (each one takes what was measured, so it can be tested without the hardware)
// ---------------------------------------------------------------------------------------------
pub struct MicFacts {
    pub voice_active: bool,
    pub text_only: bool,
    pub muted: bool,
    pub frame_age_ms: Option<u64>,
    pub error: String,
    pub devices: usize,
}

pub fn mic_check(f: &MicFacts) -> Check {
    let (name, key) = ("Микрофон", "mic");
    if f.muted {
        let mut c = check(key, name, Status::Warn, "Микрофон выключен пользователем: голосовые команды не слушаются.", "Включите микрофон: Настройки → Приватность или кнопка 🎤 на главной.");
        c.fix_id = FIX_MIC_UNMUTE;
        return c;
    }
    if f.text_only {
        return check(key, name, Status::Warn, "Джарвис запущен в режиме «только текст».", "Перезапустите Джарвиса обычным ярлыком (без --text), чтобы включить голос.");
    }
    if !f.voice_active {
        let why = if f.error.is_empty() { "голосовой режим не запущен".to_string() } else { f.error.clone() };
        return check(key, name, Status::Error, format!("Голос не работает: {}. Устройств записи: {}.", why, f.devices), "Подключите микрофон, разрешите доступ к нему в Параметры Windows → Конфиденциальность → Микрофон, выберите его в Настройках и перезапустите Джарвиса.");
    }
    match f.frame_age_ms {
        Some(age) if age < 3000 => check(key, name, Status::Ok, format!("Звук с микрофона поступает (последний блок {} мс назад).", age), ""),
        Some(age) => check(key, name, Status::Error, format!("Микрофон перестал отдавать звук {} с назад.", age / 1000), "Проверьте, не отключён ли микрофон и не занят ли другой программой; нажмите «Проверить» в Настройках → Микрофон."),
        None => check(key, name, Status::Error, "Поток открыт, но звук не поступил ни разу.", "Выберите другое устройство в Настройках → Микрофон и перезапустите Джарвиса."),
    }
}

pub fn audio_check(inputs: usize, output: Option<Result<String, String>>) -> Check {
    let (name, key) = ("Аудиосистема", "audio");
    match output {
        None => check(key, name, Status::Unchecked, format!("Устройств записи: {}. Устройство вывода проверить не удалось на этой системе.", inputs), ""),
        Some(Err(e)) => check(key, name, Status::Error, format!("Нет устройства вывода звука ({}). Устройств записи: {}.", e, inputs), "Подключите колонки или наушники и проверьте Параметры Windows → Система → Звук."),
        Some(Ok(out)) => {
            if inputs == 0 {
                check(key, name, Status::Warn, format!("Вывод звука: {}. Устройств записи не найдено.", out), "Подключите микрофон, если нужны голосовые команды.")
            } else {
                check(key, name, Status::Ok, format!("Вывод звука: {}. Устройств записи: {}.", out, inputs), "")
            }
        }
    }
}

pub enum PiperFacts {
    NotInstalled,
    Ok(u64),
    Failed(String),
    Interrupted,
}

pub fn piper_check(tts_on: bool, windows_voice: bool, test: PiperFacts, last: Option<(u64, Result<(), String>)>) -> Check {
    piper_check_play(tts_on, windows_voice, test, last, None)
}

// `play`: the last real playback. A sound file made by Piper is not proof that it was heard.
pub fn piper_check_play(tts_on: bool, windows_voice: bool, test: PiperFacts, last: Option<(u64, Result<(), String>)>, play: Option<(u64, Result<(), String>)>) -> Check {
    let (name, key) = ("Голос Piper", "piper");
    let mut last_txt = match &last {
        Some((t, Ok(()))) => format!(" Последний синтез {} успешен.", ago(*t)),
        Some((t, Err(e))) => format!(" Последняя ошибка синтеза {}: {}.", ago(*t), e),
        None => String::new(),
    };
    match &play {
        Some((t, Ok(()))) => last_txt.push_str(&format!(" Последнее воспроизведение {} прошло без ошибок.", ago(*t))),
        Some((t, Err(e))) => {
            if matches!(test, PiperFacts::Ok(_)) {
                return check(key, name, Status::Error, format!("Звук создаётся, но не воспроизводится ({}): {}.", ago(*t), e), "Проверьте устройство вывода звука в Параметры Windows → Система → Звук и громкость; нажмите «Проверить звук» в Настройках → Голос.");
            }
            last_txt.push_str(&format!(" Последняя ошибка воспроизведения {}: {}.", ago(*t), e));
        }
        None => last_txt.push_str(" Воспроизведение в этом запуске ещё не проверялось."),
    }
    let off = if tts_on { "" } else { " Озвучка ответов сейчас выключена пользователем." };
    match test {
        PiperFacts::NotInstalled => {
            if windows_voice {
                check(key, name, Status::Warn, format!("Piper не установлен; выбран голос Windows.{}", off), "Настройки → Голос → «Установить голос Джарвиса».")
            } else {
                check(key, name, Status::Error, format!("Голос Джарвиса (Piper) не установлен, ответы не звучат.{}", off), "Настройки → Голос → «Установить голос Джарвиса» (нужен интернет, ≈80 МБ).")
            }
        }
        PiperFacts::Ok(bytes) => {
            // synthesis proven; playing is proven only by a real playback of this run
            let st = if !tts_on { Status::Warn } else if play.is_some() { Status::Ok } else { Status::Unchecked };
            let fix = if !tts_on { "Включите озвучку: Настройки → Приватность." } else if play.is_none() { "Нажмите «Проверить звук» в Настройках → Голос, затем повторите проверку." } else { "" };
            check(key, name, st, format!("Пробная фраза синтезирована ({} КБ звука, без воспроизведения).{}{}", bytes / 1024, last_txt, off), fix)
        }
        PiperFacts::Failed(e) => check(key, name, Status::Error, format!("Piper не смог создать звук: {}.{}", e, last_txt), "Переустановите голос: Настройки → Голос → «Установить голос Джарвиса». Проверьте, что антивирус не блокирует piper.exe."),
        PiperFacts::Interrupted => check(key, name, Status::Unchecked, format!("Проверка прервана: Джарвис начал говорить.{}", last_txt), "Повторите проверку."),
    }
}

// the reserve voice of Windows (PowerShell + System.Speech): judged only by its own playbacks, never by Piper's
pub fn windows_voice_check(selected: bool, tts_on: bool, last: Option<(u64, Result<(), String>)>) -> Check {
    let (name, key) = ("Голос Windows (резервный)", "winvoice");
    match (selected, last) {
        (false, None) => check(key, name, Status::Unchecked, "Не выбран: Jarvis говорит голосом Piper. Голос Windows нужен только как запасной.", "Настройки → Голос → «Windows: …», если Piper не подходит."),
        (false, Some((t, Err(e)))) => check(key, name, Status::Warn, format!("Сейчас не выбран; его последняя попытка {} закончилась ошибкой: {}.", ago(t), e), "Если хотите пользоваться голосом Windows — исправьте причину; иначе оставьте голос Piper."),
        (false, Some((t, Ok(())))) => check(key, name, Status::Unchecked, format!("Сейчас не выбран; последняя попытка {} прошла без ошибок.", ago(t)), ""),
        (true, None) => check(key, name, Status::Unchecked, "Выбран, но в этом запуске ещё ничего не произносил.", "Нажмите «Проверить звук» в Настройках → Голос."),
        (true, Some((t, Ok(())))) => check(key, name, if tts_on { Status::Ok } else { Status::Warn }, format!("Последняя фраза {} прошла без ошибок (PowerShell запустился и System.Speech отработал).{}", ago(t), if tts_on { "" } else { " Озвучка выключена пользователем." }), ""),
        (true, Some((t, Err(e)))) => check(key, name, Status::Error, format!("Голос Windows не сработал ({}): {}.", ago(t), e), "Выберите голос Jarvis (Piper): Настройки → Голос → «Автоматически». Проверьте звук Windows и что PowerShell не заблокирован политикой или антивирусом."),
    }
}

// the ways of starting PowerShell for the Windows voice, each with its own answer
pub fn powershell_check(ways: &[(String, Result<(), String>)]) -> Check {
    let (name, key) = ("Запуск PowerShell (голос Windows)", "powershell");
    if ways.is_empty() {
        return check(key, name, Status::Unchecked, "Проверка только для Windows.", "");
    }
    let failed: Vec<String> = ways.iter().filter_map(|(w, r)| r.as_ref().err().map(|e| format!("«{}»: {}", w, e))).collect();
    let working: Vec<&str> = ways.iter().filter(|(_, r)| r.is_ok()).map(|(w, _)| w.as_str()).collect();
    if failed.is_empty() {
        return check(key, name, Status::Ok, format!("PowerShell запускается всеми {} способами Jarvis.", ways.len()), "");
    }
    if working.is_empty() {
        return check(key, name, Status::Error, format!("PowerShell не запускается ни одним способом. Отказы: {}.", failed.join("; ")), "Голос Windows не заработает. Выберите голос Jarvis (Piper): Настройки → Голос → «Автоматически». Проверьте, не блокирует ли PowerShell антивирус или политика (AppLocker).");
    }
    let first_ok = ways.first().map(|(_, r)| r.is_ok()).unwrap_or(false);
    check(
        key,
        name,
        if first_ok { Status::Ok } else { Status::Warn },
        format!("Работает способ «{}». Отказали: {}.", working[0], failed.join("; ")),
        if first_ok { "" } else { "Jarvis пользуется рабочим способом сам; если голос Windows всё равно молчит — выберите голос Jarvis (Piper)." },
    )
}

// the voice of Piper must be a voice of the language of Jarvis
pub fn piper_language_check(lang: &str, voice: Option<(String, bool)>) -> Option<Check> {
    match voice {
        Some((file, false)) => Some(check(
            "piper_lang",
            "Язык голоса Piper",
            Status::Warn,
            format!("Для языка «{}» нет своего голоса, Piper берёт «{}»: речь будет произноситься неверно (русские слова — английским голосом).", lang, file),
            "Настройки → Голос → «Установить голос Джарвиса» (нужен голос вашего языка) или смените язык Jarvis.",
        )),
        _ => None,
    }
}

// which program this is: the commit it was built from, and where it lies (a report of a problem must name the build)
pub fn build_check() -> Check {
    let exe = std::env::current_exe().ok();
    let when = exe
        .as_ref()
        .and_then(|p| std::fs::metadata(p).ok())
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| ago(d.as_secs()))
        .unwrap_or_else(|| "?".into());
    let (status, hint) = if crate::config::BUILD_SHA.is_some() { (Status::Ok, "") } else { (Status::Unchecked, "Сборка без метки коммита (локальная): сверить её с CI нельзя.") };
    check("build", "Установленная сборка", status, format!("Версия {}. Файл {} изменён {}.", crate::config::build_id(), exe.map(|p| p.display().to_string()).unwrap_or_default(), when), hint)
}

// the files of the commands against the list of this build (integrity.rs)
pub fn integrity_check() -> Check {
    let (name, key) = ("Файлы команд", "integrity");
    if !crate::integrity::enforced() {
        return check(key, name, Status::Unchecked, "Сборка без метки коммита (локальная): файлы команд не сверяются.", "");
    }
    let r = crate::integrity::scan();
    integrity_card(key, name, &r)
}

pub fn integrity_card(key: &'static str, name: &'static str, r: &crate::integrity::Report) -> Check {
    if r.clean() {
        return check(key, name, Status::Ok, format!("Все {} файлов команд — из этой сборки.", r.checked), "");
    }
    let list = |v: &[String]| v.iter().take(6).cloned().collect::<Vec<_>>().join(", ");
    let mut parts = Vec::new();
    if !r.modified.is_empty() {
        parts.push(format!("изменены ({}): {}", r.modified.len(), list(&r.modified)));
    }
    if !r.extra.is_empty() {
        parts.push(format!("лишние ({}): {}", r.extra.len(), list(&r.extra)));
    }
    if !r.missing.is_empty() {
        parts.push(format!("нет ({}): {}", r.missing.len(), list(&r.missing)));
    }
    check(key, name, Status::Warn, format!("Команды не совпадают с этой сборкой — {}. Такие команды работают только как простые (без запуска программ, записи файлов и клавиш).", parts.join("; ")), "Установите Jarvis заново или распакуйте commands-update.zip ТОЙ ЖЕ сборки, что и программа.")
}

pub fn ai_check(ai_enabled: bool, status: &Value) -> Check {
    let (name, key) = ("ИИ-провайдер", "ai");
    if !ai_enabled {
        return check(key, name, Status::Warn, "Запросы к ИИ выключены в Настройках → Приватность.", "Включите «Запросы к ИИ», если нужны ответы нейросети.");
    }
    let detail = status["detail"].as_str().unwrap_or("").to_string();
    if !status["configured"].as_bool().unwrap_or(false) {
        return check(key, name, Status::Warn, format!("Не настроен: {}.", detail), "Настройки → ИИ-провайдер: выберите провайдера, модель и ключ.");
    }
    let who = format!("{} · {}", status["provider"].as_str().unwrap_or(""), status["model"].as_str().unwrap_or(""));
    match status["state"].as_str().unwrap_or("") {
        "CONNECTED" => check(key, name, Status::Ok, format!("{}: последний запрос успешен{}.", who, status["last_ok"].as_u64().map(|t| format!(" ({})", ago(t))).unwrap_or_default()), ""),
        "ERROR" => check(key, name, Status::Error, format!("{}: {}.", who, detail), "Проверьте ключ, модель и интернет; нажмите «Проверить подключение» на главной."),
        "CHECKING" => check(key, name, Status::Unchecked, format!("{}: идёт проверка.", who), ""),
        // a key is saved, but nothing has proved the connection yet: no request is sent just for this indicator
        _ => check(key, name, Status::Unchecked, format!("{}: ключ задан, но подключение ещё не подтверждено реальным запросом.", who), "Нажмите «Проверить подключение» на карточке AI Connection или задайте вопрос."),
    }
}

pub fn latency_check(timing: Option<&Value>) -> Check {
    let (name, key) = ("Задержка ИИ", "ai_latency");
    match timing {
        None => check(key, name, Status::Unchecked, "Запросов к ИИ в этом запуске ещё не было.", ""),
        Some(t) => {
            let total = t["total_ms"].as_u64().unwrap_or(0);
            let first = t["first_chunk_ms"].as_u64();
            let spoke = t["first_speech_started_ms"].as_u64().map(|x| format!(", голос начал говорить через {} мс", x)).unwrap_or_default();
            let detail = format!("Последний ответ: первый фрагмент {}, целиком {} мс{}{}.", first.map(|x| format!("{} мс", x)).unwrap_or("—".into()), total, if t["streamed"].as_bool().unwrap_or(false) { " (поток)" } else { "" }, spoke);
            if total > 20_000 { check(key, name, Status::Warn, detail, "Выберите более быструю модель (например, gemini-flash) или проверьте интернет.") } else { check(key, name, Status::Ok, detail, "") }
        }
    }
}

pub fn internet_check(route: bool, dns: Result<(), String>, tcp: Result<u128, String>) -> Check {
    let (name, key) = ("Интернет", "internet");
    if !route {
        return check(key, name, Status::Error, "Нет сетевого маршрута: компьютер не подключён к сети.", "Подключитесь к Wi-Fi или кабелю.");
    }
    match (dns, tcp) {
        (Ok(()), Ok(ms)) => check(key, name, Status::Ok, format!("DNS работает, соединение с интернетом за {} мс.", ms), ""),
        (Err(e), _) => check(key, name, Status::Error, format!("Имена сайтов не определяются (DNS): {}.", e), "Проверьте подключение к интернету и настройки DNS/прокси."),
        (Ok(()), Err(e)) => check(key, name, Status::Warn, format!("DNS работает, но соединение не установлено: {}.", e), "Возможно, интернет закрыт брандмауэром или прокси."),
    }
}

pub fn remote_check(running: bool, lan_answer: Option<Result<(), String>>, last_client: Option<u64>) -> Check {
    let (name, key) = ("Пульт телефона", "remote");
    if !running {
        return check(key, name, Status::Unchecked, "Пульт выключен.", "Включите пульт на главной, если нужен телефон.");
    }
    match (lan_answer, last_client) {
        (Some(Err(e)), _) => check(key, name, Status::Error, format!("Пульт включён, но не отвечает по адресу в сети: {}.", e), "Выключите и включите пульт; разрешите Джарвиса в брандмауэре Windows (частные сети)."),
        (_, Some(t)) => check(key, name, Status::Ok, format!("Сервер отвечает в локальной сети; устройство с правильной ссылкой подключалось {}.", ago(t)), ""),
        (Some(Ok(())), None) => check(key, name, Status::Unchecked, "Сервер отвечает по адресу в локальной сети, но телефон ещё не подключался — доступ с телефона не подтверждён.", "Откройте ссылку или QR-код на телефоне в той же Wi-Fi сети. Если не открывается — разрешите Джарвиса в брандмауэре Windows."),
        (None, None) => check(key, name, Status::Unchecked, "Не удалось определить адрес в сети для проверки.", ""),
    }
}

// the Telegram bot: PASS only when Telegram really answered the polling of the bot just now (the status says so)
pub fn telegram_check(enabled: bool, token_saved: bool, paired: bool, status: &str) -> Check {
    let (name, key) = ("Telegram-бот", "telegram");
    if !enabled {
        return check(key, name, Status::Unchecked, "Бот выключен: связь с Telegram не проверялась.", "Включите бота в Настройках → Управление через Telegram, если он нужен.");
    }
    if !token_saved {
        return check(key, name, Status::Error, "Бот включён, но токен не сохранён.", "Вставьте токен от @BotFather и сохраните его.");
    }
    if status != "работает" {
        return check(key, name, Status::Error, format!("Telegram не отвечает боту: {}.", status), "Проверьте интернет, токен у @BotFather и что бота не опрашивает другая программа.");
    }
    if !paired {
        return check(key, name, Status::Warn, "Бот на связи с Telegram, но телефон не привязан: команды никто не отправит.", "Нажмите «Получить код» и отправьте код боту.");
    }
    check(key, name, Status::Ok, "Бот на связи с Telegram и привязан к вашему аккаунту. Выполнение команд из Telegram проверяйте вручную (см. чек-лист).", "")
}

// the access from any network: PASS only when the public address really opened from this computer
pub fn tunnel_check(state: &str, error: &str, opened: Option<Result<(), String>>) -> Check {
    let (name, key) = ("Доступ из любой сети (туннель)", "tunnel");
    match state {
        "off" => check(key, name, Status::Unchecked, "Туннель выключен (так задумано: по умолчанию пульт открыт только в вашей сети).", "Включите на главной в блоке «Пульт», если нужен доступ не из вашего Wi-Fi."),
        "downloading" | "starting" => check(key, name, Status::Warn, "Туннель запускается.", "Подождите полминуты и запустите проверку ещё раз."),
        "error" => check(key, name, Status::Error, format!("Туннель не запустился: {}.", error), "Проверьте интернет; антивирус или брандмауэр мог заблокировать cloudflared."),
        _ => match opened {
            Some(Ok(())) => check(key, name, Status::Ok, "Публичный адрес выдан и страница пульта открылась по нему с этого компьютера. С телефона вне сети проверьте вручную.", ""),
            Some(Err(e)) => check(key, name, Status::Error, format!("Адрес выдан, но страница по нему не открылась: {}.", e), "Выключите и включите туннель."),
            None => check(key, name, Status::Unchecked, "Адрес выдан, но открыть его для проверки не удалось.", ""),
        },
    }
}

// the notifications of other programs (they are read only with the permission of the user and of Windows)
pub fn notify_check(enabled: bool, state: &str, detail: &str) -> Check {
    let (name, key) = ("Уведомления других программ", "ext_notify");
    if !enabled {
        return check(key, name, Status::Unchecked, "Чтение выключено (по умолчанию): Jarvis не видит уведомления других программ.", "Включите в Настройках → Уведомления других программ, если нужны мессенджеры.");
    }
    match state {
        "on" => check(key, name, Status::Ok, "Windows разрешила чтение, наблюдатель работает. Что приложения реально присылают уведомления, зависит от них и от браузера.", ""),
        "starting" => check(key, name, Status::Warn, "Наблюдатель запускается.", "Запустите проверку ещё раз через несколько секунд."),
        "denied" => check(key, name, Status::Error, "Windows не дала доступ к уведомлениям.", if detail.is_empty() { "Параметры Windows → Конфиденциальность → Уведомления → разрешите доступ приложениям.".to_string() } else { detail.to_string() }),
        "unsupported" => check(key, name, Status::Unchecked, "Эта функция работает только в Windows.", ""),
        _ => check(key, name, Status::Error, format!("Наблюдатель не работает{}.", if detail.is_empty() { String::new() } else { format!(": {}", detail) }), "Выключите и снова включите чтение уведомлений."),
    }
}

// the game mode (gamemode.rs): the state, the anti-cheat guard, the list, the last scan; nothing personal
fn measure_aim() -> Check {
    let cfg = crate::gamemode::aim_config();
    aim_check(&crate::aimmodel::status_json(&cfg), &crate::aimassist::status_json(&cfg))
}

pub fn game_check(f: &crate::gamemode::Facts) -> Check {
    let (name, key) = ("Игровой режим", "game_mode");
    let facts = format!(
        "Режим {} · игр в списке {} · макросов {} · метка {} · звук {}{}.",
        if f.active { if f.in_game { "включён, окно игры впереди" } else { "включён, окно игры не впереди" } } else { "выключен" },
        f.games,
        f.macros,
        onoff(f.overlay),
        onoff(f.sound),
        match (f.scan_running, f.scan) {
            (true, _) => " · поиск игр идёт".to_string(),
            (_, Some((found, added, skipped))) => format!(" · последний поиск: найдено {}, добавлено {}, не добавлено {}", found, added, skipped),
            _ => " · поиск игр ещё не запускали".to_string(),
        }
    );
    if let Some(ac) = &f.anticheat {
        return check(key, name, Status::Warn, format!("Запущена защита от читов ({}): режим не включается и не работает, это защита от случайного вреда. {}", ac, facts), "Закройте игру с защитой от читов. Режим только для одиночных игр без неё.");
    }
    if !f.active && f.games == 0 && f.scan.is_none() {
        return check(key, name, Status::Unchecked, format!("Режим не использовался. {}", facts), "Откройте раздел «Игры» и нажмите «Найти мои игры» или добавьте игру вручную.");
    }
    check(key, name, Status::Ok, format!("{} Работу в живой игре эта проверка не подтверждает: проверяйте её в самой игре.", facts), "")
}

// the aim assist (aimmodel.rs, aimassist.rs): the model, its checks, the runtime, the device, the capture; `st` is aimmodel::status_json, `run` is aimassist::status_json
pub fn aim_check(st: &serde_json::Value, run: &serde_json::Value) -> Check {
    let (name, key) = ("Помощь прицела", "aim");
    let s = |v: &serde_json::Value, k: &str| v[k].as_str().unwrap_or("").to_string();
    if !st["runtime"].as_bool().unwrap_or(false) {
        return check(key, name, Status::Unchecked, "В этой сборке нет ONNX Runtime: помощь прицела недоступна.", "Используйте полный установщик Jarvis для Windows.");
    }
    let facts = format!(
        "Среда выполнения: ONNX Runtime встроен (Python не нужен) · устройство: {} · DirectML в сборке: {} · захват: {} · кадр {} мс · людей в кадре {}",
        if s(run, "device_used").is_empty() { "модель ещё не запускалась".to_string() } else { s(run, "device_used") },
        if st["directml_build"].as_bool().unwrap_or(false) { "да" } else { "нет" },
        s(run, "capture"),
        run["last_frame_ms"],
        run["objects"]
    );
    if let Some(p) = Some(s(run, "problem")).filter(|p| !p.is_empty()) {
        return check(key, name, Status::Error, format!("Помощь прицела остановилась: {}. {}", p, facts), "Исправьте причину и включите помощь прицела снова.");
    }
    if !st["installed"].as_bool().unwrap_or(false) {
        let why = s(st, "install_error");
        return check(key, name, Status::Unchecked, format!("Модель не установлена{}. {}", if why.is_empty() { String::new() } else { format!(" (последняя попытка: {})", why) }, facts), "Раздел «Игры» → «Помощь прицела» → «Установить модель автоматически» или «Выбрать файл модели…».");
    }
    let src = if st["meta"]["kind"] == "official" { "официальная загрузка, SHA-256 и размер сверены" } else { "файл пользователя, эталона для SHA-256 нет" };
    let model = format!("Модель: {} ({} МБ), {}", st["meta"]["name"].as_str().unwrap_or("без описания"), st["size_mb"], src);
    if st["checked"].as_bool().unwrap_or(false) && !st["check_ok"].as_bool().unwrap_or(false) {
        return check(key, name, Status::Error, format!("{}. Проверка совместимости не пройдена: {}. {}", model, s(st, "check_error"), facts), "Установите другую модель или нажмите «Проверить модель» после замены файла.");
    }
    if !st["checked"].as_bool().unwrap_or(false) {
        return check(key, name, Status::Warn, format!("{}. Совместимость ещё не проверена. {}", model, facts), "Нажмите «Проверить модель» в карточке.");
    }
    check(key, name, Status::Ok, format!("{}. Совместимость: пройдена ({}, вход {}). {}. Точность в живой игре эта проверка не подтверждает.", model, st["summary"]["layout"].as_str().unwrap_or(""), st["summary"]["input"].as_str().unwrap_or(""), facts), "")
}

// the life of the program: its folder of data can be written, autostart points to this copy
pub fn lifecycle_check(data_dir: Result<(), String>, autostart_on: bool, autostart_other_copy: bool, mem_mb: Option<f64>) -> Check {
    let (name, key) = ("Жизненный цикл программы", "lifecycle");
    if let Err(e) = data_dir {
        return check(key, name, Status::Error, format!("Папка данных Jarvis недоступна для записи: {}. Настройки, диалоги и токены не сохранятся.", e), "Проверьте права на папку %APPDATA%\\com.priler.jarvis и свободное место.");
    }
    if autostart_on && autostart_other_copy {
        return check(key, name, Status::Warn, "Автозапуск включён, но указывает на другую копию Jarvis.", "Выключите и снова включите автозагрузку в настройках — она запишет эту копию.");
    }
    let mem = mem_mb.map(|m| format!(" Jarvis занимает {:.0} МБ памяти.", m)).unwrap_or_default();
    check(key, name, Status::Ok, format!("Папка данных доступна для записи; автозапуск {}.{} Закрытие, трей и обновление поверх запущенной программы проверяются в CI установщика.", if autostart_on { "включён" } else { "выключен" }, mem), "")
}

// the stream of sound from the microphone (wakewatch.rs): flowing, paused on purpose, silent, or stalled
pub fn watch_check(w: &Value) -> Check {
    let (name, key) = ("Поток звука с микрофона", "mic_watch");
    let text = w["text"].as_str().unwrap_or("").to_string();
    match w["state"].as_str().unwrap_or("") {
        "ok" => check(key, name, Status::Ok, format!("{}. Это проверка потока звука, а не распознавания слова «Джарвис».", text), ""),
        "paused" => check(key, name, Status::Unchecked, text, "Включите микрофон в трее или командой «включи микрофон»."),
        "silent" => check(key, name, Status::Warn, text, "Параметры Windows → Звук → Ввод: проверьте уровень и что микрофон не отключён."),
        _ if w["gave_up"].as_bool().unwrap_or(false) => check(key, name, Status::Error, text, "Подключите микрофон заново и перезапустите Джарвиса."),
        _ => check(key, name, Status::Warn, text, "Подождите несколько секунд: Джарвис переоткрывает микрофон сам."),
    }
}

// the password of the owner and the permissions switched off
pub fn security_check(owner_set: bool, unlocked: bool, off: &[&str]) -> Check {
    let (name, key) = ("Защита и разрешения", "security");
    let perms = if off.is_empty() { "Все способности включены по умолчанию.".to_string() } else { format!("Выключено в центре разрешений: {}.", off.join(", ")) };
    if owner_set {
        check(key, name, Status::Ok, format!("Пароль владельца задан, сейчас {}. {}", if unlocked { "разблокировано" } else { "заблокировано" }, perms), "")
    } else {
        check(key, name, Status::Ok, format!("Пароль владельца не задан: отправка сообщений и удаление данных доступны без пароля. {}", perms), "Задайте пароль в Настройки → Безопасность и данные, если компьютером пользуются другие.")
    }
}

pub fn scheduler_check(total: usize, enabled: usize, next_in_secs: Option<u64>) -> Check {
    let (name, key) = ("Расписание заданий", "scheduler");
    let next = next_in_secs.map(|s| if s < 90 { "ближайшее меньше чем через минуту".to_string() } else if s < 5400 { format!("ближайшее через {} мин", s / 60) } else { format!("ближайшее через {} ч", s / 3600) }).unwrap_or_else(|| "ближайшего запуска нет".into());
    check(key, name, Status::Ok, format!("Заданий: {}, включено: {}; {}.", total, enabled, next), "")
}

pub fn smarthome_check(configured: bool, permission_on: bool) -> Check {
    let (name, key) = ("Умный дом", "smarthome");
    match (configured, permission_on) {
        (false, _) => check(key, name, Status::Unchecked, "Home Assistant не подключён (необязательно).", ""),
        (true, false) => check(key, name, Status::Warn, "Адрес и токен сохранены, но управление выключено в центре разрешений.", "Включите «Управлять умным домом» в Настройки → Безопасность и данные."),
        (true, true) => check(key, name, Status::Ok, "Подключение настроено. Доступность сервера проверяется кнопкой «Показать устройства».", ""),
    }
}

pub fn monitor_check(mem_total_gb: f32, cpu: f32, disk_total_gb: f32) -> Check {
    let (name, key) = ("Системный мониторинг", "monitor");
    if mem_total_gb <= 0.0 {
        return check(key, name, Status::Error, "Показатели памяти не получены.", "Перезапустите Джарвиса; если не поможет — пришлите диагностический отчёт.");
    }
    let disk = if disk_total_gb > 0.0 { format!(", диск {:.0} ГБ", disk_total_gb) } else { ", диск: нет данных".into() };
    let st = if disk_total_gb > 0.0 { Status::Ok } else { Status::Warn };
    check(key, name, st, format!("Память {:.1} ГБ, процессор {:.0}%{}.", mem_total_gb, cpu, disk), "")
}

pub fn resources_check(commands: usize, missing: &[String], vosk: bool) -> Check {
    let (name, key) = ("Файлы и ресурсы", "files");
    if commands == 0 {
        return check(key, name, Status::Error, format!("Команды не загружены.{}", if missing.is_empty() { String::new() } else { format!(" Нет: {}.", missing.join(", ")) }), "Переустановите Джарвиса (папка resources повреждена или удалена).");
    }
    if !missing.is_empty() {
        return check(key, name, Status::Error, format!("Загружено команд: {}. Нет: {}.", commands, missing.join(", ")), "Переустановите Джарвиса или распакуйте commands-update.zip в папку resources.");
    }
    if !vosk {
        return check(key, name, Status::Warn, format!("Загружено команд: {}. Модель распознавания речи (Vosk) не найдена — голос недоступен.", commands), "Установите полный установщик (Jarvis-Setup.exe) или модель из resources/vosk/README.md.");
    }
    check(key, name, Status::Ok, format!("Загружено команд: {}; звуки, модель речи и команды на месте.", commands), "")
}

pub fn commands_check(found: &[(&str, bool)]) -> Check {
    let (name, key) = ("Системные команды", "syscmd");
    let missing: Vec<&str> = found.iter().filter(|(_, ok)| !ok).map(|(n, _)| *n).collect();
    if found.is_empty() {
        return check(key, name, Status::Unchecked, "Проверка доступна только в Windows.", "");
    }
    if missing.is_empty() {
        check(key, name, Status::Ok, format!("Найдены: {}.", found.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")), "")
    } else {
        check(key, name, Status::Error, format!("Не найдены: {}.", missing.join(", ")), "Без них часть команд не работает (PowerShell — озвучка, громкость, офис). Проверьте политику компьютера или восстановите компоненты Windows.")
    }
}

// ---------------------------------------------------------------------------------------------
// measuring (the only place that touches the system; nothing here changes anything)
// ---------------------------------------------------------------------------------------------
fn with_timeout<T: Send + 'static>(ms: u64, f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_millis(ms)).ok()
}

fn measure_internet() -> Check {
    let (route, _) = crate::health::network();
    let dns = with_timeout(3000, || ("www.msftconnecttest.com", 443).to_socket_addrs().map(|mut a| a.next()).map_err(|e| e.to_string()))
        .unwrap_or_else(|| Err("нет ответа за 3 с".into()));
    let (dns_res, addr) = match dns {
        Ok(Some(a)) => (Ok(()), Some(a)),
        Ok(None) => (Err("адрес не найден".to_string()), None),
        Err(e) => (Err(e), None),
    };
    let tcp = match addr {
        Some(a) => {
            let t0 = std::time::Instant::now();
            TcpStream::connect_timeout(&a, Duration::from_secs(3)).map(|_| t0.elapsed().as_millis()).map_err(|e| e.to_string())
        }
        None => Err("не проверено".into()),
    };
    internet_check(route, dns_res, tcp)
}

// The remote page answers on the address of the computer in the local network (without the secret: the answer is
// "forbidden", which proves that the server listens there; nothing is executed).
fn measure_remote() -> Check {
    let running = crate::remote::is_running();
    let lan = if running {
        let url = crate::remote::url();
        let host = url.trim_start_matches("https://").trim_start_matches("http://").split('/').next().unwrap_or("").to_string();
        if host.is_empty() {
            None
        } else {
            Some(
                with_timeout(3000, move || -> Result<(), String> {
                    let addr = host.to_socket_addrs().map_err(|e| e.to_string())?.next().ok_or("нет адреса")?;
                    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
                    s.set_read_timeout(Some(Duration::from_secs(2))).ok();
                    write!(s, "GET /diag HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n", host).map_err(|e| e.to_string())?;
                    let mut buf = [0u8; 16];
                    let n = s.read(&mut buf).map_err(|e| e.to_string())?;
                    if buf[..n].starts_with(b"HTTP/1.") { Ok(()) } else { Err("непонятный ответ".into()) }
                })
                .unwrap_or_else(|| Err("нет ответа за 3 с".into())),
            )
        }
    } else {
        None
    };
    remote_check(running, lan, crate::remote::last_client())
}

#[cfg(feature = "lua")]
fn output_device() -> Option<Result<String, String>> {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};
    with_timeout(3000, || match rodio::cpal::default_host().default_output_device() {
        Some(d) => Ok(d.name().unwrap_or_else(|_| "устройство по умолчанию".into())),
        None => Err("устройство по умолчанию не найдено".into()),
    })
}

#[cfg(not(feature = "lua"))]
fn output_device() -> Option<Result<String, String>> {
    None
}

fn measure_telegram() -> Check {
    let j = crate::telegram::json();
    telegram_check(j["enabled"] == true, j["token_saved"] == true, j["paired"] == true, j["status"].as_str().unwrap_or(""))
}

fn measure_notify() -> Check {
    let j = crate::notify::ext_status_json();
    notify_check(crate::uistate::settings().ext_enabled, j["state"].as_str().unwrap_or(""), j["detail"].as_str().unwrap_or(""))
}

fn measure_tunnel() -> Check {
    let j = crate::tunnel::json();
    let state = j["state"].as_str().unwrap_or("off").to_string();
    let opened = if state == "on" { open_public_page() } else { None };
    tunnel_check(&state, j["error"].as_str().unwrap_or(""), opened)
}

// the page of the remote opened through the public address (the page holds no secret): the real way the phone goes
#[cfg(feature = "lua")]
fn open_public_page() -> Option<Result<(), String>> {
    let url = match crate::tunnel::state() {
        crate::tunnel::State::On(u) => u,
        _ => return None,
    };
    // a tunnel that was just made needs a few seconds before its address answers: three tries
    with_timeout(30_000, move || -> Result<(), String> {
        let c = reqwest::blocking::Client::builder().timeout(Duration::from_secs(6)).build().map_err(|_| "нет сети".to_string())?;
        let mut last = String::from("нет ответа");
        for k in 0..3 {
            if k > 0 {
                std::thread::sleep(Duration::from_secs(3));
            }
            match c.get(&url).send() {
                Ok(r) if r.status().is_success() => return Ok(()),
                Ok(r) => last = format!("ответ {}", r.status().as_u16()),
                Err(_) => last = "нет ответа".into(),
            }
        }
        Err(last)
    })
}

#[cfg(not(feature = "lua"))]
fn open_public_page() -> Option<Result<(), String>> {
    None
}

fn measure_lifecycle() -> Check {
    let dir = crate::APP_CONFIG_DIR.get().cloned();
    let writable = match dir {
        Some(d) => {
            let probe = d.join(format!("diag_probe_{}.tmp", std::process::id()));
            let r = crate::create_fresh(&probe).and_then(|mut f| std::io::Write::write_all(&mut f, b"ok")).map_err(|e| e.to_string());
            let _ = std::fs::remove_file(&probe);
            r
        }
        None => Err("папка не определена".into()),
    };
    let a = crate::autostart::state();
    lifecycle_check(writable, a["on"] == true, a["other_copy"] == true, crate::health::self_memory_mb())
}

fn measure_resources() -> Check {
    let app = &*crate::APP_DIR;
    let mut missing = Vec::new();
    for (path, what) in [("resources/commands", "папка команд"), ("resources/sound", "звуки"), ("resources/commands/_prelude.lua", "общие функции команд")] {
        if !app.join(path).exists() {
            missing.push(what.to_string());
        }
    }
    let vosk = std::fs::read_dir(app.join(crate::config::VOSK_MODELS_PATH)).map(|rd| rd.flatten().any(|e| e.path().is_dir())).unwrap_or(false);
    resources_check(crate::COMMANDS_COUNT.load(Ordering::Relaxed), &missing, vosk)
}

fn measure_syscmds() -> Check {
    #[cfg(target_os = "windows")]
    {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let sys = std::path::Path::new(&root).join("System32");
        let list = [
            ("PowerShell", sys.join("WindowsPowerShell\\v1.0\\powershell.exe")),
            ("cmd", sys.join("cmd.exe")),
            ("explorer", std::path::Path::new(&root).join("explorer.exe")),
            ("shutdown", sys.join("shutdown.exe")),
            ("rundll32", sys.join("rundll32.exe")),
        ];
        let found: Vec<(&str, bool)> = list.iter().map(|(n, p)| (*n, p.exists())).collect();
        commands_check(&found)
    }
    #[cfg(not(target_os = "windows"))]
    commands_check(&[])
}

fn os_version() -> String {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        if let Ok(out) = std::process::Command::new("cmd").args(["/C", "ver"]).creation_flags(0x0800_0000).output() {
            let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !v.is_empty() {
                return v;
            }
        }
        "Windows".into()
    }
    #[cfg(not(target_os = "windows"))]
    format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
}

// ---------------------------------------------------------------------------------------------
// running and the report
// ---------------------------------------------------------------------------------------------
#[derive(Clone)]
struct Run {
    at: u64,
    checks: Vec<Check>,
    os: String,
}

static LAST: Lazy<Mutex<Option<Run>>> = Lazy::new(|| Mutex::new(None));
static RUNNING: AtomicBool = AtomicBool::new(false);

pub fn run() -> Vec<Check> {
    use std::sync::atomic::Ordering::SeqCst;
    let devices = crate::recorder::get_audio_devices();
    let mic = mic_check(&MicFacts {
        voice_active: crate::VOICE_ACTIVE.load(SeqCst),
        text_only: crate::VOICE_TEXT_ONLY.load(SeqCst),
        muted: crate::MIC_MUTED.load(SeqCst),
        frame_age_ms: crate::recorder::last_frame_age_ms(),
        error: crate::VOICE_ERROR.lock().clone(),
        devices: devices.len(),
    });
    // no microphone in this PC, but the remote on a phone or a tablet is used: that is a normal setup, not a fault
    let mic = if mic.status == Status::Error && crate::uistate::voice_via_remote() {
        check("mic", "Микрофон", Status::Ok, "На этом компьютере микрофона нет: голос принимается через пульт на телефоне или планшете (он подключался недавно).", "")
    } else {
        mic
    };
    let audio = audio_check(devices.len(), output_device());
    let test = match crate::tts::synth_check() {
        crate::tts::SynthCheck::NotInstalled => PiperFacts::NotInstalled,
        crate::tts::SynthCheck::Ok(b) => PiperFacts::Ok(b),
        crate::tts::SynthCheck::Failed(e) => PiperFacts::Failed(e),
        crate::tts::SynthCheck::Interrupted => PiperFacts::Interrupted,
    };
    let piper = piper_check_play(crate::tts::is_enabled(), crate::uistate::voice_name().starts_with("win:"), test, crate::tts::last_synth(), crate::tts::last_play_for(crate::tts::Engine::Piper));
    let windows_voice = windows_voice_check(crate::uistate::voice_name().starts_with("win:"), crate::tts::is_enabled(), crate::tts::last_play_for(crate::tts::Engine::Windows));
    let build = build_check();
    let powershell = powershell_check(&crate::tts::probe_powershell_ways());
    let piper_lang = piper_language_check(&crate::i18n::get_language(), crate::tts::piper_voice_info());
    let ai_status = crate::ai::status_json();
    let ai = ai_check(crate::uistate::settings().ai_enabled, &ai_status);
    let latency = latency_check(ai_status.get("timing").filter(|t| !t.is_null()));
    let internet = measure_internet();
    let remote = measure_remote();
    let st = crate::uistate::stats();
    let monitor = monitor_check(st.mem_total_gb, st.cpu, st.disk_total_gb);
    let off: Vec<&str> = crate::perms::ALL.iter().filter(|c| !crate::perms::allowed(**c)).map(|c| c.title()).collect();
    let jobs = crate::scheduler::list();
    let next = jobs.iter().filter(|j| j.enabled).map(|j| j.at).min().map(|t| t.saturating_sub(crate::store::now()));
    let mut extra = vec![watch_check(&crate::wakewatch::status_json()), security_check(crate::owner::is_set(), crate::owner::unlocked(), &off)];
    if !jobs.is_empty() {
        extra.push(scheduler_check(jobs.len(), jobs.iter().filter(|j| j.enabled).count(), next));
    }
    if crate::smarthome::is_configured() {
        extra.push(smarthome_check(true, crate::perms::allowed(crate::perms::Cap::SmartHome)));
    }
    let mut checks = vec![build, integrity_check(), mic, audio, piper, windows_voice, powershell, ai, latency, internet, remote, measure_tunnel(), measure_telegram(), measure_notify(), monitor, measure_resources(), measure_syscmds(), measure_lifecycle(), game_check(&crate::gamemode::facts()), measure_aim()];
    checks.extend(extra);
    if let Some(c) = piper_lang {
        // (right after the card of Piper)
        let at = checks.iter().position(|c| c.key == "piper").map(|i| i + 1).unwrap_or(checks.len());
        checks.insert(at, c);
    }
    *LAST.lock() = Some(Run { at: now(), checks: checks.clone(), os: os_version() });
    checks
}

// started from the window: in the background, the window asks for the result
pub fn start() -> bool {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return false;
    }
    std::thread::spawn(|| {
        let _ = std::panic::catch_unwind(run);
        RUNNING.store(false, Ordering::SeqCst);
    });
    true
}

pub fn json() -> Value {
    let last = LAST.lock().clone();
    json!({
        "running": RUNNING.load(Ordering::SeqCst),
        "at": last.as_ref().map(|r| r.at),
        "checks": last.as_ref().map(|r| r.checks.clone()).unwrap_or_default(),
        "warm_piper": crate::tts::warm_status(),
        "events": safe_events(100),
    })
}

// an event of the page; "op" (the number of the operation, the same as «вся операция» in the security journal) only when it has one
fn event_json(time: u64, kind: &str, text: &str, op: u64) -> Value {
    let mut v = json!({ "time": time, "kind": kind, "text": text });
    if op > 0 {
        v["op"] = json!(op);
    }
    v
}

// events of the program that carry no personal text: start, voice, AI state, errors. Events that quote what the user
// said («…») and notifications are left out.
fn safe_events(n: usize) -> Vec<Value> {
    let base: Vec<Value> = crate::uistate::recent_events(80)
        .into_iter()
        .filter(|e| matches!(e.kind.as_str(), "start" | "voice" | "ai" | "error"))
        .filter(|e| !e.text.contains('«') && !e.text.starts_with("Уведомление") && !e.text.starts_with("Вопрос"))
        .map(|e| event_json(e.time, &e.kind, &sanitize(&e.text), e.op))
        .collect();
    merge_mic_events(base, crate::voicediag::recent(40), n)
}

// the microphone journal (voicediag.rs: states and errors of the audio path, never sound or words) joins the events as «voice»,
// newest first
pub fn merge_mic_events(mut base: Vec<Value>, mic: Vec<crate::voicediag::Event>, n: usize) -> Vec<Value> {
    for e in mic {
        base.push(event_json(e.at, "voice", &format!("Микрофон: {}", sanitize(&e.text)), e.op));
    }
    base.sort_by(|a, b| b["time"].as_u64().unwrap_or(0).cmp(&a["time"].as_u64().unwrap_or(0)));
    base.truncate(n);
    base
}

// removes anything that looks like a secret: the saved AI key, the remote token, keys and tokens in URLs and headers
pub fn sanitize(text: &str) -> String {
    let mut out = text.to_string();
    let key = crate::uistate::settings().ai_key;
    if key.len() >= 6 {
        out = out.replace(&key, "***");
    }
    let token = crate::remote::token_for_diag();
    if token.len() >= 6 {
        out = out.replace(&token, "***");
    }
    // the secret of the window server too (it is in the address that opens the window)
    if let Some(t) = crate::ui_auth::token() {
        out = out.replace(&t, "***");
    }
    let mut prev_bearer = false;
    let words: Vec<String> = out
        .split(' ')
        .map(|w| {
            let lw = w.to_lowercase();
            if std::mem::replace(&mut prev_bearer, lw == "bearer") && !w.is_empty() {
                return "***".to_string();
            }
            let secretish = lw.starts_with("sk-") || lw.starts_with("aiza") || lw.starts_with("sk_") || lw.starts_with("xox") || lw.starts_with("ghp_");
            if secretish && w.len() > 12 {
                return "***".to_string();
            }
            for marker in ["t=", "token=", "key=", "api_key=", "apikey=", "password="] {
                if let Some(i) = lw.find(marker) {
                    let start = i + marker.len();
                    if w.len() > start && w.is_char_boundary(start) {
                        return format!("{}***", &w[..start]);
                    }
                }
            }
            w.to_string()
        })
        .collect();
    words.join(" ")
}

// the text of "Copy the diagnostics report": the last results only (nothing is run, nothing is sent)
pub fn report() -> String {
    let last = LAST.lock().clone();
    let s = crate::uistate::settings();
    let ai = crate::ai::status_json();
    let mut r = String::new();
    r.push_str("=== Диагностический отчёт Jarvis ===\n");
    r.push_str(&format!("Версия Jarvis: {}\n", crate::config::APP_VERSION.unwrap_or("?")));
    r.push_str(&format!("Система: {}\n", last.as_ref().map(|l| l.os.clone()).unwrap_or_else(os_version_cached)));
    r.push_str(&format!("Язык: {}\n", crate::i18n::get_language()));
    r.push_str(&format!("ИИ: провайдер {} · модель {} · запросы {} · состояние {}\n", ai["provider"].as_str().filter(|p| !p.is_empty()).unwrap_or("не выбран"), ai["model"].as_str().filter(|p| !p.is_empty()).unwrap_or("—"), if s.ai_enabled { "включены" } else { "выключены" }, ai["state"].as_str().unwrap_or("")));
    r.push_str(&format!("Озвучка: {} · голос: {} · микрофон: {}\n", if crate::tts::is_enabled() { "вкл" } else { "выкл" }, crate::tts::voice_label(&s.voice_name), if crate::MIC_MUTED.load(Ordering::SeqCst) { "выключен" } else { "включён" }));
    r.push_str(&format!("Уведомления: показ {} · голос {} · звук {} · не беспокоить {} · внешние {} ({})\n", onoff(s.notify_show), onoff(s.notify_speak), onoff(s.notify_sound), onoff(s.dnd), onoff(s.ext_enabled), crate::notify::ext_status_json()["state"].as_str().unwrap_or("")));
    match last {
        None => r.push_str("\nПроверка ещё не запускалась (нажмите «Проверить всё»).\n"),
        Some(l) => {
            r.push_str(&format!("\nПроверка: {} (unix {})\n", ago(l.at), l.at));
            for c in &l.checks {
                r.push_str(&format!("[{}] {}: {}\n", c.status.text(), c.name, sanitize(&c.detail)));
                if !c.fix.is_empty() && c.status != Status::Ok {
                    r.push_str(&format!("    Что сделать: {}\n", c.fix));
                }
            }
        }
    }
    let ev = safe_events(15);
    if !ev.is_empty() {
        r.push_str("\nПоследние события (без личных данных):\n");
        for e in ev {
            r.push_str(&format!("  {} [{}] {}\n", e["time"], e["kind"].as_str().unwrap_or(""), e["text"].as_str().unwrap_or("")));
        }
    }
    r.push_str("\nВ отчёт не входят: ключи API, токены, пароли, тексты диалогов и уведомлений.\n");
    r
}

fn os_version_cached() -> String {
    format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
}

fn onoff(b: bool) -> &'static str {
    if b { "вкл" } else { "выкл" }
}

// ---------------------------------------------------------------------------------------------
// what uses the Internet (the privacy section): from the real settings
// ---------------------------------------------------------------------------------------------
pub fn internet_uses(s: &crate::uistate::UiSettings) -> Value {
    let provider_local = matches!(s.ai_provider.as_str(), "ollama") || (s.ai_provider == "custom" && {
        let u = s.ai_url.to_lowercase();
        u.contains("://localhost") || u.contains("://127.") || u.contains("://192.168.") || u.contains("://10.")
    });
    let ai_active = s.ai_enabled && s.ai_provider != "none";
    let ai_detail = if !s.ai_enabled {
        "выключено в разделе Приватность: ничего не отправляется".to_string()
    } else if s.ai_provider == "none" {
        "провайдер не выбран: ничего не отправляется".to_string()
    } else if provider_local {
        "сервер на этом компьютере или в локальной сети: в интернет не уходит".to_string()
    } else {
        "ваши вопросы и последние сообщения диалога уходят выбранному провайдеру, когда вы их задаёте; команды компьютера не отправляются".to_string()
    };
    json!([
        { "name": "Ответы ИИ", "internet": ai_active && !provider_local, "active": ai_active, "detail": ai_detail },
        { "name": "Анализ экрана", "internet": s.ai_screen && ai_active && !provider_local, "active": s.ai_screen && ai_active, "detail": if s.ai_screen { "снимок экрана уходит ИИ-провайдеру только по команде «что на экране» и не сохраняется" } else { "выключено: снимки экрана никуда не отправляются" } },
        { "name": "Погода", "internet": true, "active": !s.city.is_empty(), "detail": "название города уходит на open-meteo.com (блок на главной) и wttr.in (команда «какая погода»)" },
        { "name": "Поиск, сайты, карты, перевод, музыка", "internet": true, "active": true, "detail": "открываются в браузере, только когда вы просите" },
        { "name": "Установка голоса Джарвиса", "internet": true, "active": false, "detail": "загрузка с github.com и huggingface.co, только по кнопке" },
        { "name": "Распознавание речи (Vosk)", "internet": false, "active": true, "detail": "на этом компьютере: звук с микрофона никуда не отправляется" },
        { "name": "Голос (Piper)", "internet": false, "active": true, "detail": "на этом компьютере" },
        { "name": "Пульт телефона", "internet": false, "active": crate::remote::is_running(), "detail": "по умолчанию только локальная сеть (Wi-Fi), по секретной ссылке; из других сетей — только с включённым туннелем (строка ниже)" },
        { "name": "Пульт из любой сети (туннель Cloudflare)", "internet": true, "active": matches!(crate::tunnel::state(), crate::tunnel::State::On(_)), "detail": "только если вы включили в карточке пульта: трафик пульта идёт через серверы Cloudflare; программа cloudflared скачивается один раз с github.com" },
        { "name": "Управление через Telegram", "internet": true, "active": crate::telegram::json()["enabled"] == true, "detail": "только если вы включили бота: тексты команд и ответов идут через api.telegram.org; бот слушается одного привязанного аккаунта" },
        { "name": "Уведомления других программ", "internet": false, "active": s.ext_enabled, "detail": "читаются только на этом компьютере, никуда не отправляются и не сохраняются" },
        { "name": "Диагностика", "internet": true, "active": false, "detail": "проверка интернета подключается к msftconnecttest.com, только по кнопке «Проверить всё»" },
    ])
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod new_checks {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_microphone_stream_is_judged_by_what_wakewatch_says() {
        assert_eq!(watch_check(&json!({"state":"ok","text":"Звук идёт"})).status, Status::Ok);
        assert_eq!(watch_check(&json!({"state":"paused","text":"выкл"})).status, Status::Unchecked);
        assert_eq!(watch_check(&json!({"state":"silent","text":"тишина"})).status, Status::Warn);
        assert_eq!(watch_check(&json!({"state":"stalled","text":"нет звука","gave_up":false})).status, Status::Warn);
        assert_eq!(watch_check(&json!({"state":"stalled","text":"нет звука","gave_up":true})).status, Status::Error);
        assert!(watch_check(&json!({"state":"ok","text":"x"})).detail.contains("а не распознавания"));
    }

    #[test]
    fn security_scheduler_and_smart_home_say_what_is_true() {
        let c = security_check(false, true, &["Писать сообщения контактам"]);
        assert!(c.detail.contains("не задан") && c.detail.contains("Выключено") && !c.fix.is_empty());
        assert!(security_check(true, false, &[]).detail.contains("заблокировано"));
        assert!(scheduler_check(3, 2, Some(300)).detail.contains("через 5 мин"));
        assert!(scheduler_check(0, 0, None).detail.contains("нет"));
        assert_eq!(smarthome_check(false, false).status, Status::Unchecked);
        assert_eq!(smarthome_check(true, false).status, Status::Warn);
        assert_eq!(smarthome_check(true, true).status, Status::Ok);
    }
}

// ---------------------------------------------------------------------------------------------
// fixes the owner can confirm. A FIXED list: each one is one named change of Jarvis' own state (never a command, a path or text from outside).
// The window first asks for the preview, the owner reads it and confirms; without the confirmation nothing changes. There is no «fix all».
// ---------------------------------------------------------------------------------------------
pub const FIX_MIC_UNMUTE: &str = "mic_unmute";

// what the fix will do, in words, and whether it still applies to the state of the program now
pub fn fix_preview(id: &str) -> Option<String> {
    match id {
        FIX_MIC_UNMUTE => Some(if crate::MIC_MUTED.load(Ordering::SeqCst) {
            "Включить микрофон в Jarvis: голосовые команды снова будут слушаться. Настройки Windows и других программ не меняются.".to_string()
        } else {
            "Микрофон в Jarvis уже включён: менять нечего.".to_string()
        }),
        _ => None,
    }
}

pub fn fix_apply(id: &str, confirmed: bool) -> Value {
    let Some(preview) = fix_preview(id) else {
        return json!({ "ok": false, "message": "Такого исправления нет." });
    };
    if !confirmed {
        crate::audit::record("diag_fix", id, "medium", "declined", false, "владелец не подтвердил");
        return json!({ "ok": false, "needs_confirm": true, "preview": preview, "message": "Исправление не выполнено: нужно подтверждение." });
    }
    let done = match id {
        FIX_MIC_UNMUTE if crate::MIC_MUTED.load(Ordering::SeqCst) => {
            crate::uistate::set_mic_muted(false);
            true
        }
        _ => false,
    };
    crate::audit::record("diag_fix", id, "medium", if done { "applied" } else { "nothing_to_do" }, true, "подтверждено владельцем");
    crate::uistate::log_event("info", &format!("Исправление «{}»: {}", id, if done { "выполнено" } else { "менять нечего" }));
    json!({ "ok": true, "applied": done, "message": if done { "Готово." } else { "Менять нечего." } })
}

#[cfg(test)]
mod fix_tests {
    use super::*;

    #[test]
    fn only_the_muted_microphone_offers_a_fix_and_the_events_show_their_operation_number() {
        let facts = |muted| MicFacts { voice_active: true, text_only: false, muted, frame_age_ms: Some(10), error: String::new(), devices: 1 };
        assert_eq!(mic_check(&facts(true)).fix_id, FIX_MIC_UNMUTE);
        assert_eq!(mic_check(&facts(false)).fix_id, "");
        // an event without an operation has no "op", with one it has
        assert!(event_json(5, "info", "x", 0).get("op").is_none());
        assert_eq!(event_json(5, "info", "x", 7)["op"], 7);
        // the microphone journal carries the number too
        let e = crate::voicediag::Event { at: 9, kind: "device".into(), text: "x".into(), op: 3 };
        assert_eq!(merge_mic_events(vec![], vec![e], 5)[0]["op"], 3);
    }
}
