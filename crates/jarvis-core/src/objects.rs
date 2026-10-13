// Object Interaction Engine: what Jarvis is working with right now (a Word document, a table, a text file, a code file,
// a presentation, a folder, a page) and in which state the work is.
//
// This module is the single place that knows
//   * the typed state machine (IDLE, APP_OPEN, OBJECT_ACTIVE, DICTATION, AI_TASK, AWAITING_CONFIRMATION, PAUSED, ERROR_RECOVERY)
//     with explicit, checkable transitions (`next`) - a forbidden transition is an error, never a silent change;
//   * the context of the object (id, kind, program, how the window was found, path, mode, operation, pending question,
//     AI task, short history without secrets, results, the fact of a confirmed save);
//   * the voice control words of the work ("стоп", "пауза", "отмена", "продолжим с объектом") and where they apply:
//     inside a dictation only a whole short phrase is a command, any longer phrase is the dictated text;
//   * the guards: a result of a cancelled AI task is never accepted; the window is checked before an action.
//
// It does not start programs, write files or recognise speech: the adapters (office.lua / office.ps1, codeobj.rs) do
// that through the existing safe paths (psrun, safefs, policy), the speech comes from the existing pipeline.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------------------------
// the state machine
// ---------------------------------------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle,
    AppOpen,
    ObjectActive,
    Dictation,
    AiTask,
    AwaitingConfirmation,
    Paused,
    ErrorRecovery,
}

impl State {
    pub const ALL: [State; 8] = [State::Idle, State::AppOpen, State::ObjectActive, State::Dictation, State::AiTask, State::AwaitingConfirmation, State::Paused, State::ErrorRecovery];

    pub fn name(self) -> &'static str {
        match self {
            State::Idle => "IDLE",
            State::AppOpen => "APP_OPEN",
            State::ObjectActive => "OBJECT_ACTIVE",
            State::Dictation => "DICTATION",
            State::AiTask => "AI_TASK",
            State::AwaitingConfirmation => "AWAITING_CONFIRMATION",
            State::Paused => "PAUSED",
            State::ErrorRecovery => "ERROR_RECOVERY",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    OpenApp,         // the program is open (no object yet)
    BindObject,      // a document / file / folder is chosen and verified
    StartDictation,
    EndDictation,
    StartAi,
    AiDone,
    AiCancelled,
    AiFailed,
    AskConfirm,
    ConfirmYes,
    ConfirmNo,
    Pause,           // "стоп" / "пауза": keeps the context
    Resume,          // "продолжим": only after the object was checked again
    ObjectLost,      // the window / document / file is gone
    AppClosed,       // the program was closed during the work
    RecoverOk,       // the error is handled
    Release,         // the work with the object is finished normally (the program stays open)
    Reset,           // forget everything
}

// A transition is allowed only when it is in this table. Everything else is `Err(Forbidden)`.
pub fn next(from: State, ev: Event) -> Result<State, Forbidden> {
    use Event::*;
    use State::*;
    let to = match (from, ev) {
        (_, Reset) => Idle,

        (Idle, OpenApp) => AppOpen,
        (Idle, BindObject) => ObjectActive, // the program was open before Jarvis looked at it

        (AppOpen, OpenApp) => AppOpen,
        (AppOpen, BindObject) => ObjectActive,
        (AppOpen, AppClosed) => Idle,
        (AppOpen, Release) => Idle,

        (ObjectActive, BindObject) => ObjectActive, // another object of the same program: the old context is replaced
        (ObjectActive, StartDictation) => Dictation,
        (ObjectActive, StartAi) => AiTask,
        (ObjectActive, AskConfirm) => AwaitingConfirmation,
        (ObjectActive, Pause) => Paused,
        (ObjectActive, ObjectLost) | (ObjectActive, AppClosed) => ErrorRecovery,
        (ObjectActive, Release) => AppOpen,

        (Dictation, EndDictation) => ObjectActive,
        (Dictation, StartAi) => AiTask, // the dictation is ended first (incompatible operations are not mixed)
        (Dictation, AskConfirm) => AwaitingConfirmation,
        (Dictation, Pause) => Paused,
        (Dictation, ObjectLost) | (Dictation, AppClosed) => ErrorRecovery,
        (Dictation, Release) => AppOpen,

        (AiTask, AiDone) | (AiTask, AiCancelled) => ObjectActive,
        (AiTask, AiFailed) => ErrorRecovery,
        (AiTask, AskConfirm) => AwaitingConfirmation, // the result replaces text: ask first
        (AiTask, Pause) => Paused,                    // the generation is cancelled, its result is dropped
        (AiTask, ObjectLost) | (AiTask, AppClosed) => ErrorRecovery,

        (AwaitingConfirmation, ConfirmYes) | (AwaitingConfirmation, ConfirmNo) => ObjectActive,
        (AwaitingConfirmation, Pause) => Paused, // a question never survives a pause: the old "да" must not run a stale action
        (AwaitingConfirmation, ObjectLost) | (AwaitingConfirmation, AppClosed) => ErrorRecovery,

        (Paused, Resume) => ObjectActive,
        (Paused, ObjectLost) | (Paused, AppClosed) => ErrorRecovery,
        (Paused, Release) => AppOpen,

        (ErrorRecovery, RecoverOk) => ObjectActive,
        (ErrorRecovery, OpenApp) => AppOpen,
        (ErrorRecovery, BindObject) => ObjectActive,
        (ErrorRecovery, Release) => AppOpen,

        (from, ev) => return Err(Forbidden { from, ev }),
    };
    Ok(to)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Forbidden {
    pub from: State,
    pub ev: Event,
}

impl std::fmt::Display for Forbidden {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "переход {:?} запрещён в состоянии {}", self.ev, self.from.name())
    }
}

// ---------------------------------------------------------------------------------------------
// the object and its context
// ---------------------------------------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    WordDoc,
    ExcelBook,
    TextFile,
    CodeFile,
    Presentation,
    Folder,
    BrowserPage,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::WordDoc => "word_doc",
            Kind::ExcelBook => "excel_book",
            Kind::TextFile => "text_file",
            Kind::CodeFile => "code_file",
            Kind::Presentation => "presentation",
            Kind::Folder => "folder",
            Kind::BrowserPage => "browser_page",
        }
    }

    // a spoken name for the replies ("документ", "таблица" ...)
    pub fn spoken(self) -> &'static str {
        match self {
            Kind::WordDoc => "документ",
            Kind::ExcelBook => "таблица",
            Kind::TextFile => "заметка",
            Kind::CodeFile => "файл",
            Kind::Presentation => "презентация",
            Kind::Folder => "папка",
            Kind::BrowserPage => "страница",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        [Kind::WordDoc, Kind::ExcelBook, Kind::TextFile, Kind::CodeFile, Kind::Presentation, Kind::Folder, Kind::BrowserPage].into_iter().find(|k| k.name() == s)
    }

    // the kind of object a program works with
    pub fn for_app(app: &str) -> Option<Kind> {
        match app {
            "word" => Some(Kind::WordDoc),
            "excel" => Some(Kind::ExcelBook),
            "notepad" => Some(Kind::TextFile),
            "vscode" => Some(Kind::CodeFile),
            "powerpoint" => Some(Kind::Presentation),
            "explorer" => Some(Kind::Folder),
            "browser" => Some(Kind::BrowserPage),
            _ => None,
        }
    }
}

// How it was found out that the window in front really is the object (never "assumed")
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detect {
    None,
    ProcessOnly,    // the program in front is the right one, the document name is unknown
    ProcessTitle,   // the program and the document name in the title
    ComDocument,    // the program itself reported its active document (Office automation)
}

impl Detect {
    pub fn name(self) -> &'static str {
        match self {
            Detect::None => "none",
            Detect::ProcessOnly => "process",
            Detect::ProcessTitle => "process+title",
            Detect::ComDocument => "office-com",
        }
    }

    pub fn parse(s: &str) -> Detect {
        match s {
            "process" => Detect::ProcessOnly,
            "process+title" => Detect::ProcessTitle,
            "office-com" => Detect::ComDocument,
            _ => Detect::None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiStatus {
    None,
    Running,
    Done,
    Cancelled,
    Failed,
}

impl AiStatus {
    pub fn name(self) -> &'static str {
        match self {
            AiStatus::None => "none",
            AiStatus::Running => "running",
            AiStatus::Done => "done",
            AiStatus::Cancelled => "cancelled",
            AiStatus::Failed => "failed",
        }
    }
}

const MAX_RECENT: usize = 8;
const MAX_RESULTS: usize = 5;
const MAX_TEXT: usize = 240;

#[derive(Debug, Clone)]
pub struct Message {
    pub role: &'static str, // "user" | "jarvis"
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct Object {
    pub id: String,
    pub kind: Kind,
    pub app: String,            // "word", "excel", "notepad", "vscode" ...
    pub name: String,           // the name of the document / file shown to the user
    pub path: Option<String>,   // a full path, when the object has one
    pub project: Option<String>, // the folder of the project (VS Code) or the directory (Explorer)
    pub detect: Detect,
}

#[derive(Debug, Clone)]
pub struct Pending {
    pub id: String,       // what is asked ("overwrite", "replace_text", "delete", "save_as")
    pub question: String, // the exact question that was asked (also the thing the user agreed to)
    pub created: u64,
}

#[derive(Debug, Clone)]
pub struct Context {
    pub object: Option<Object>,
    pub mode: String,            // "dictation" | "edit" | "view" | ""
    pub operation: String,       // the running operation ("dictation", "ai:код", "save" ...)
    pub pending: Option<Pending>,
    pub ai: AiStatus,
    pub ai_label: String,
    pub recent: VecDeque<Message>,
    pub results: VecDeque<String>,
    pub saved_confirmed: bool,   // true only after the adapter verified that the file really exists / was saved
    pub saved_path: Option<String>,
    pub error: String,           // the reason of the ERROR_RECOVERY state
    pub resume_prompted: bool,   // "Работа приостановлена. Продолжим?" was asked and waits for да / нет
    pub updated: u64,
}

impl Context {
    fn empty() -> Context {
        Context {
            object: None,
            mode: String::new(),
            operation: String::new(),
            pending: None,
            ai: AiStatus::None,
            ai_label: String::new(),
            recent: VecDeque::new(),
            results: VecDeque::new(),
            saved_confirmed: false,
            saved_path: None,
            error: String::new(),
            resume_prompted: false,
            updated: now(),
        }
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// A text that may be kept in the history of the object: no keys, tokens or passwords, not long.
pub fn sanitize(text: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut hide_next = false;
    for w in text.split_whitespace() {
        let bare = w.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_');
        let lower = bare.to_lowercase();
        if hide_next {
            hide_next = false;
            words.push("[скрыто]".to_string());
            continue;
        }
        if ["пароль", "password", "passwd", "токен", "token", "ключ", "apikey", "api_key", "secret", "секрет", "pin", "пин"].contains(&lower.as_str()) {
            words.push(w.to_string());
            hide_next = true;
            continue;
        }
        if looks_like_secret(bare) {
            words.push("[скрыто]".to_string());
            continue;
        }
        words.push(w.to_string());
    }
    let joined = words.join(" ");
    if joined.chars().count() > MAX_TEXT {
        let cut: String = joined.chars().take(MAX_TEXT).collect();
        format!("{}…", cut)
    } else {
        joined
    }
}

// a long run of letters/digits that looks like a key (sk-..., AIza..., a hex hash, a bot token with a colon)
fn looks_like_secret(w: &str) -> bool {
    let n = w.chars().count();
    if n >= 20 && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ':') && w.chars().any(|c| c.is_ascii_digit()) {
        return true;
    }
    for p in ["sk-", "sk_", "AIza", "ghp_", "gho_", "xox", "hf_"] {
        if w.starts_with(p) && n >= 12 {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------------------------
// the engine
// ---------------------------------------------------------------------------------------------
pub struct Engine {
    state: State,
    ctx: Context,
    seq: u64,
    task_seq: u64,
    task: Option<(u64, Arc<AtomicBool>)>, // the running AI task and its cancel flag
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Forbidden(Forbidden),
    NoObject,
    Busy(&'static str),
}

impl Error {
    pub fn user_text(&self) -> String {
        match self {
            Error::Forbidden(f) => format!("Так нельзя: {}.", f),
            Error::NoObject => "Сейчас нет объекта, с которым я работаю.".to_string(),
            Error::Busy(what) => format!("Сначала нужно завершить: {}.", what),
        }
    }
}

// What a stop of the work could really do. Never "everything is stopped" when something could not be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopReport {
    pub was: State,
    pub dictation_stopped: bool,
    pub ai_cancelled: bool,    // the generation was running and the cancel flag is set (its result will be dropped)
    pub question_dropped: bool,
    pub paused: bool,
    pub object_name: String,
}

impl Engine {
    pub fn new() -> Engine {
        Engine { state: State::Idle, ctx: Context::empty(), seq: 0, task_seq: 0, task: None }
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn context(&self) -> &Context {
        &self.ctx
    }

    fn go(&mut self, ev: Event) -> Result<(), Error> {
        let to = next(self.state, ev).map_err(Error::Forbidden)?;
        self.state = to;
        self.ctx.updated = now();
        Ok(())
    }

    fn clear_op(&mut self) {
        self.ctx.operation.clear();
        self.ctx.mode.clear();
    }

    pub fn reset(&mut self) {
        self.cancel_task();
        self.state = State::Idle;
        self.ctx = Context::empty();
    }

    // the program is open, no object yet
    pub fn open_app(&mut self, app: &str) -> Result<(), Error> {
        self.go(Event::OpenApp)?;
        // another program than before: the old object is not this program's object
        if self.ctx.object.as_ref().map_or(false, |o| o.app != app) {
            self.ctx = Context::empty();
        }
        Ok(())
    }

    // an object is chosen (and checked by the adapter). The context of the previous object is replaced.
    pub fn bind(&mut self, app: &str, kind: Kind, name: &str, path: Option<&str>, project: Option<&str>, detect: Detect) -> Result<String, Error> {
        if matches!(self.state, State::AiTask | State::AwaitingConfirmation) {
            return Err(Error::Busy(if self.state == State::AiTask { "генерацию ИИ" } else { "вопрос, ожидающий ответа" }));
        }
        self.go(Event::BindObject)?;
        self.seq += 1;
        let id = format!("obj-{}", self.seq);
        self.cancel_task();
        self.ctx = Context::empty();
        self.ctx.object = Some(Object {
            id: id.clone(),
            kind,
            app: app.to_string(),
            name: name.to_string(),
            path: path.map(|p| p.to_string()),
            project: project.map(|p| p.to_string()),
            detect,
        });
        Ok(id)
    }

    pub fn object(&self) -> Option<&Object> {
        self.ctx.object.as_ref()
    }

    pub fn start_dictation(&mut self) -> Result<(), Error> {
        self.need_object()?;
        self.go(Event::StartDictation)?;
        self.ctx.mode = "dictation".to_string();
        self.ctx.operation = "dictation".to_string();
        Ok(())
    }

    pub fn end_dictation(&mut self) -> Result<(), Error> {
        self.go(Event::EndDictation)?;
        self.clear_op();
        Ok(())
    }

    fn need_object(&self) -> Result<(), Error> {
        if self.ctx.object.is_some() {
            Ok(())
        } else {
            Err(Error::NoObject)
        }
    }

    // ---- AI task: one at a time, always cancellable, a late result is never accepted ----
    // Returns the number of the task and the flag the generation must watch.
    pub fn start_ai(&mut self, label: &str) -> Result<(u64, Arc<AtomicBool>), Error> {
        self.need_object()?;
        if self.state == State::AiTask {
            return Err(Error::Busy("предыдущую генерацию"));
        }
        self.go(Event::StartAi)?;
        self.task_seq += 1;
        let flag = Arc::new(AtomicBool::new(false));
        self.task = Some((self.task_seq, flag.clone()));
        self.ctx.ai = AiStatus::Running;
        self.ctx.ai_label = sanitize(label);
        self.ctx.mode = "edit".to_string();
        self.ctx.operation = format!("ai:{}", self.ctx.ai_label);
        Ok((self.task_seq, flag))
    }

    fn cancel_task(&mut self) -> bool {
        match self.task.take() {
            Some((_, flag)) => {
                flag.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    // The generation finished. `true` = the result may be used; `false` = it is dropped (cancelled, paused, stale).
    pub fn ai_finished(&mut self, task: u64, result: Result<String, String>) -> bool {
        let current = matches!(&self.task, Some((n, flag)) if *n == task && !flag.load(Ordering::SeqCst));
        if !current || self.state != State::AiTask {
            // a cancelled / replaced / paused task: whatever it produced is not inserted anywhere
            if matches!(&self.task, Some((n, _)) if *n == task) {
                self.task = None;
            }
            return false;
        }
        self.task = None;
        match result {
            Ok(summary) => {
                let _ = self.go(Event::AiDone);
                self.ctx.ai = AiStatus::Done;
                self.push_result(&summary);
                self.clear_op();
                true
            }
            Err(why) => {
                let _ = self.go(Event::AiFailed);
                self.ctx.ai = AiStatus::Failed;
                self.ctx.error = sanitize(&why);
                self.clear_op();
                false
            }
        }
    }

    // ---- a question that waits for "да" / "нет" ----
    pub fn ask(&mut self, id: &str, question: &str) -> Result<(), Error> {
        self.need_object()?;
        if self.state == State::AwaitingConfirmation {
            return Err(Error::Busy("предыдущий вопрос"));
        }
        self.go(Event::AskConfirm)?;
        self.ctx.pending = Some(Pending { id: id.to_string(), question: sanitize(question), created: now() });
        self.ctx.operation = format!("confirm:{}", id);
        Ok(())
    }

    // The answer. `Some(id)` = the user agreed to exactly this pending operation: the caller runs it once.
    pub fn answer(&mut self, yes: bool) -> Result<Option<String>, Error> {
        if self.state != State::AwaitingConfirmation {
            return Err(Error::Forbidden(Forbidden { from: self.state, ev: if yes { Event::ConfirmYes } else { Event::ConfirmNo } }));
        }
        let pending = self.ctx.pending.take();
        self.go(if yes { Event::ConfirmYes } else { Event::ConfirmNo })?;
        self.clear_op();
        Ok(if yes { pending.map(|p| p.id) } else { None })
    }

    // ---- stop / pause / cancel ----
    // "Стоп", "остановись", "хватит": ends the dictation / cancels the generation / drops the question and keeps the
    // object (the program and the document stay open). The context stays for "продолжим".
    pub fn stop(&mut self) -> StopReport {
        let was = self.state;
        let mut rep = StopReport {
            was,
            dictation_stopped: was == State::Dictation,
            ai_cancelled: false,
            question_dropped: was == State::AwaitingConfirmation,
            paused: false,
            object_name: self.ctx.object.as_ref().map(|o| o.name.clone()).unwrap_or_default(),
        };
        if was == State::AiTask {
            rep.ai_cancelled = self.cancel_task();
            if rep.ai_cancelled {
                self.ctx.ai = AiStatus::Cancelled;
            }
        }
        if matches!(was, State::ObjectActive | State::Dictation | State::AiTask | State::AwaitingConfirmation) && self.go(Event::Pause).is_ok() {
            self.ctx.pending = None;
            self.clear_op();
            self.ctx.resume_prompted = true;
            rep.paused = true;
        }
        rep
    }

    // "Пауза": the same as the stop, but Jarvis does not ask "Продолжим?" - the user says when to go on.
    pub fn pause(&mut self) -> StopReport {
        let rep = self.stop();
        self.ctx.resume_prompted = false;
        rep
    }

    // "Отмена": only the running operation (the generation, the question) is cancelled. The object stays active.
    pub fn cancel(&mut self) -> Option<&'static str> {
        match self.state {
            State::AiTask => {
                self.cancel_task();
                self.ctx.ai = AiStatus::Cancelled;
                let _ = self.go(Event::AiCancelled);
                self.clear_op();
                Some("генерация отменена")
            }
            State::AwaitingConfirmation => {
                self.ctx.pending = None;
                let _ = self.go(Event::ConfirmNo);
                self.clear_op();
                Some("вопрос отменён, ничего не изменено")
            }
            State::Dictation => {
                let _ = self.go(Event::EndDictation);
                self.clear_op();
                Some("диктовка остановлена")
            }
            _ => None,
        }
    }

    // The adapter checked the object again (program running, document the same, file not changed): go on.
    pub fn resume(&mut self) -> Result<(), Error> {
        self.need_object()?;
        self.go(Event::Resume)?;
        self.ctx.resume_prompted = false;
        Ok(())
    }

    // The user said "нет" to "Продолжим?": nothing is closed or deleted, the context stays for a later "продолжим".
    pub fn decline_resume(&mut self) {
        self.ctx.resume_prompted = false;
    }

    // ---- losses and errors ----
    pub fn lost(&mut self, why: &str) -> Result<(), Error> {
        self.cancel_task();
        self.go(Event::ObjectLost)?;
        self.ctx.error = sanitize(why);
        self.ctx.pending = None;
        self.clear_op();
        Ok(())
    }

    pub fn app_closed(&mut self) -> Result<(), Error> {
        self.cancel_task();
        self.go(Event::AppClosed)?;
        self.ctx.error = "программа закрыта во время работы".to_string();
        self.ctx.pending = None;
        self.clear_op();
        Ok(())
    }

    pub fn recovered(&mut self) -> Result<(), Error> {
        self.go(Event::RecoverOk)?;
        self.ctx.error.clear();
        Ok(())
    }

    // The work with the object ends from any state: the running operation is cancelled first (a question is answered
    // "нет", a generation is cancelled), then the program and the document stay open as they are.
    pub fn finish(&mut self) -> Result<(), Error> {
        match self.state {
            State::AiTask | State::AwaitingConfirmation => {
                self.cancel();
            }
            _ => {}
        }
        self.release()
    }

    // the work is finished normally ("заканчиваем работу с объектом"): the program and the document stay open
    pub fn release(&mut self) -> Result<(), Error> {
        self.cancel_task();
        self.go(Event::Release)?;
        self.ctx.pending = None;
        self.clear_op();
        self.ctx.resume_prompted = false;
        Ok(())
    }

    // ---- facts ----
    pub fn saved(&mut self, path: &str) {
        self.ctx.saved_confirmed = true;
        self.ctx.saved_path = Some(path.to_string());
        self.push_result(&format!("сохранено: {}", path));
    }

    // a change after the save: the save is no longer a fact about the current text
    pub fn changed(&mut self) {
        self.ctx.saved_confirmed = false;
    }

    pub fn note(&mut self, role: &'static str, text: &str) {
        let t = sanitize(text);
        if t.is_empty() {
            return;
        }
        self.ctx.recent.push_back(Message { role, text: t });
        while self.ctx.recent.len() > MAX_RECENT {
            self.ctx.recent.pop_front();
        }
    }

    fn push_result(&mut self, text: &str) {
        self.ctx.results.push_back(sanitize(text));
        while self.ctx.results.len() > MAX_RESULTS {
            self.ctx.results.pop_front();
        }
    }

    pub fn snapshot(&self) -> Value {
        let o = self.ctx.object.as_ref();
        json!({
            "state": self.state.name(),
            "active": self.state != State::Idle,
            "app": o.map(|o| o.app.clone()).unwrap_or_default(),
            "kind": o.map(|o| o.kind.name()).unwrap_or(""),
            "name": o.map(|o| o.name.clone()).unwrap_or_default(),
            "path": o.and_then(|o| o.path.clone()).unwrap_or_default(),
            "project": o.and_then(|o| o.project.clone()).unwrap_or_default(),
            "detect": o.map(|o| o.detect.name()).unwrap_or("none"),
            "mode": self.ctx.mode,
            "operation": self.ctx.operation,
            "ai": self.ctx.ai.name(),
            "pending": self.ctx.pending.as_ref().map(|p| p.question.clone()).unwrap_or_default(),
            "saved": self.ctx.saved_confirmed,
            "saved_path": self.ctx.saved_path.clone().unwrap_or_default(),
            "last_result": self.ctx.results.back().cloned().unwrap_or_default(),
            "error": self.ctx.error,
            "resume_prompted": self.ctx.resume_prompted,
        })
    }
}

impl Default for Engine {
    fn default() -> Self {
        Engine::new()
    }
}

// ---------------------------------------------------------------------------------------------
// the window really in front
// ---------------------------------------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Front {
    pub process: String, // "winword.exe" (lower case)
    pub title: String,
}

pub trait Probe {
    // the normal window in front (not Jarvis itself), or None
    fn front(&self) -> Option<Front>;
    // is a window of the program present at all (the object may be open but not in front)
    fn running(&self, process: &str) -> bool;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verify {
    Ok(Detect),
    WrongWindow { found: String },
    Lost,
}

pub fn process_of(app: &str) -> Option<&'static str> {
    match app {
        "word" => Some("winword.exe"),
        "excel" => Some("excel.exe"),
        "notepad" => Some("notepad.exe"),
        "vscode" => Some("code.exe"),
        "powerpoint" => Some("powerpnt.exe"),
        "explorer" => Some("explorer.exe"),
        _ => None,
    }
}

// the title without the extension and the program tail, lower case
fn stem(name: &str) -> String {
    let n = name.trim().to_lowercase();
    match n.rsplit_once('.') {
        Some((a, ext)) if ext.len() <= 5 && !a.is_empty() => a.to_string(),
        _ => n,
    }
}

// Is the window in front the object? A text must never land in another window: when it is not the object, the answer
// is `WrongWindow` (the program is open but something else is in front) or `Lost` (the program is not there).
pub fn verify(obj: &Object, probe: &dyn Probe) -> Verify {
    let proc = match process_of(&obj.app) {
        Some(p) => p,
        None => return Verify::Lost,
    };
    if !probe.running(proc) {
        return Verify::Lost;
    }
    let front = match probe.front() {
        Some(f) => f,
        None => return Verify::WrongWindow { found: String::new() },
    };
    if front.process != proc {
        return Verify::WrongWindow { found: front.process };
    }
    let want = stem(&obj.name);
    if obj.name.is_empty() || obj.app == "notepad" && want.is_empty() {
        return Verify::Ok(Detect::ProcessOnly);
    }
    if front.title.to_lowercase().contains(&want) {
        Verify::Ok(Detect::ProcessTitle)
    } else {
        Verify::WrongWindow { found: front.title }
    }
}

// ---------------------------------------------------------------------------------------------
// the voice control words
// ---------------------------------------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Stop,
    Pause,
    Cancel,
    Resume,
    Yes,
    No,
}

impl Control {
    pub fn name(self) -> &'static str {
        match self {
            Control::Stop => "stop",
            Control::Pause => "pause",
            Control::Cancel => "cancel",
            Control::Resume => "resume",
            Control::Yes => "yes",
            Control::No => "no",
        }
    }
}

const NOISE: &[&str] = &["джарвис", "jarvis", "пожалуйста", "ну", "так"];

const STOP_PHRASES: &[&str] = &[
    "стоп", "остановись", "остановите", "остановить", "хватит", "прекрати", "прекрати ввод", "прекратить ввод", "пока остановись", "стоп диктовка", "стоп диктовку",
    "остановить диктовку", "останови диктовку", "прекрати диктовку", "хватит диктовать", "стоп ввод", "останови ввод", "не закрывай просто остановись",
    "не закрывай только остановись", "просто остановись", "стоп генерация", "останови генерацию", "хватит писать",
];
const PAUSE_PHRASES: &[&str] = &["пауза", "поставь на паузу", "поставить на паузу", "приостанови", "приостановить", "приостанови работу", "пауза работа"];
const CANCEL_PHRASES: &[&str] = &["отмена", "отмени", "отменить", "отмени это", "отмени операцию", "отмена операции", "отмени генерацию", "отмени задачу", "отмени действие", "отмена действия"];
const YES: &[&str] = &["да", "ага", "угу", "конечно", "верно", "согласен", "согласна", "окей", "ок", "подтверждаю", "да продолжим", "да продолжай", "давай"];
const NO: &[&str] = &["нет", "не надо", "не нужно", "неа", "не хочу", "потом", "не сейчас"];

// "продолжим с объектом", "продолжай с этим файлом", "вернись к документу", "продолжай писать код", "возобнови работу"
fn is_resume(words: &[String]) -> bool {
    if words.is_empty() || words.len() > 6 {
        return false;
    }
    let first = words[0].as_str();
    let verb = ["продолж", "возобнов", "вернис", "вернемс", "вернуться"].iter().any(|s| first.starts_with(s));
    if !verb {
        return false;
    }
    if words.len() == 1 {
        return true;
    }
    // the rest must be about the work, not another command ("продолжай воспроизведение")
    let rest = &words[1..];
    let ok = ["с", "к", "со", "этим", "этому", "этой", "этого", "объектом", "объекту", "объект", "файлом", "файлу", "документом", "документу", "работу", "работой", "писать", "код", "кода", "кодом",
        "диктовку", "диктовать", "таблицей", "таблице", "таблицу", "заметкой", "заметку", "презентацией", "нему", "ней", "дальше", "там", "где", "остановились", "закончили", "пока", "вернись"];
    rest.iter().all(|w| ok.contains(&w.as_str()))
}

fn matches_set(words: &[String], set: &[&str]) -> bool {
    let joined = words.join(" ");
    set.iter().any(|p| *p == joined)
}

// What the phrase means for the work with the object. `None` = not a control word: in a dictation it is the text,
// elsewhere it is an ordinary command. The state decides where a word applies:
//   * DICTATION: only the WHOLE phrase "стоп" / "пауза" / "отмена" ... is a command; "он сказал стоп и ушёл" is text;
//   * AWAITING_CONFIRMATION: да / нет / отмена;
//   * PAUSED, ERROR_RECOVERY: продолжим / да / нет (and again стоп / пауза are accepted: nothing to stop, but not text);
//   * AI_TASK, OBJECT_ACTIVE, APP_OPEN: стоп / пауза / отмена / продолжим.
pub fn classify(text: &str, state: State) -> Option<Control> {
    let words: Vec<String> = crate::apps::tokens(text).into_iter().filter(|w| !NOISE.contains(&w.as_str())).collect();
    if words.is_empty() {
        return None;
    }
    let stop = matches_set(&words, STOP_PHRASES);
    let pause = matches_set(&words, PAUSE_PHRASES);
    let cancel = matches_set(&words, CANCEL_PHRASES);
    let yes = matches_set(&words, YES);
    let no = matches_set(&words, NO);
    match state {
        State::Idle => None,
        State::Dictation => {
            if stop {
                Some(Control::Stop)
            } else if pause {
                Some(Control::Pause)
            } else if cancel {
                Some(Control::Cancel)
            } else {
                None
            }
        }
        State::AwaitingConfirmation => {
            if yes {
                Some(Control::Yes)
            } else if no {
                Some(Control::No)
            } else if cancel {
                Some(Control::Cancel)
            } else if stop {
                Some(Control::Stop)
            } else if pause {
                Some(Control::Pause)
            } else {
                None
            }
        }
        State::Paused | State::ErrorRecovery => {
            if is_resume(&words) {
                Some(Control::Resume)
            } else if yes {
                Some(Control::Yes)
            } else if no {
                Some(Control::No)
            } else if stop || pause || cancel {
                Some(Control::Stop)
            } else {
                None
            }
        }
        State::AiTask | State::ObjectActive | State::AppOpen => {
            if stop {
                Some(Control::Stop)
            } else if pause {
                Some(Control::Pause)
            } else if cancel {
                Some(Control::Cancel)
            } else if is_resume(&words) {
                Some(Control::Resume)
            } else {
                None
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// the voice answers (short, honest)
// ---------------------------------------------------------------------------------------------
pub fn stop_reply(rep: &StopReport, prompt_resume: bool) -> String {
    let what = if rep.object_name.is_empty() { "объектом".to_string() } else { format!("«{}»", rep.object_name) };
    if !rep.paused {
        return match rep.was {
            State::Paused => "Работа уже приостановлена.".to_string(),
            State::Idle | State::AppOpen => "Сейчас нечего останавливать.".to_string(),
            _ => "Не удалось остановить работу.".to_string(),
        };
    }
    let mut parts: Vec<String> = Vec::new();
    if rep.dictation_stopped {
        parts.push("Диктовка остановлена".to_string());
    }
    if rep.ai_cancelled {
        parts.push("Генерация отменена, её результат не будет вставлен".to_string());
    }
    if rep.question_dropped {
        parts.push("Вопрос отменён, ничего не изменено".to_string());
    }
    if parts.is_empty() {
        parts.push("Остановился".to_string());
    }
    let head = format!("{}.", parts.join(". "));
    if prompt_resume {
        format!("{} Работа с {} приостановлена. Продолжим?", head, what)
    } else {
        format!("{} Работа с {} на паузе.", head, what)
    }
}

// ---------------------------------------------------------------------------------------------
// the process-wide engine (one object at a time) and the thin functions the app and the scripts use
// ---------------------------------------------------------------------------------------------
static ENGINE: Lazy<Mutex<Engine>> = Lazy::new(|| Mutex::new(Engine::new()));

pub fn with<R>(f: impl FnOnce(&mut Engine) -> R) -> R {
    f(&mut ENGINE.lock())
}

pub fn state() -> State {
    ENGINE.lock().state()
}

pub fn snapshot() -> Value {
    ENGINE.lock().snapshot()
}

// the control word for the phrase in the current state of the engine
pub fn control_for(text: &str) -> Option<Control> {
    classify(text, state())
}

#[cfg(test)]
pub static TEST_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

#[cfg(test)]
mod tests;
