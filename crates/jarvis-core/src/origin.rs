// Where a command came from.
//
// Every phrase that reaches the command handler travels as an `Incoming`: the text AND the entrance it came through.
// The entrance is set by the code of that entrance (the phone server, the window server, the Telegram bot, the
// microphone loop ...), never read from the text or from a field of the request: a user cannot name his own source
// ("source=voice", a control character at the start, a JSON field), because nothing in the text is ever parsed as one.
// The only way to put a phrase into the queue is `remote::submit(source, text)`, so an entrance without a source
// does not compile.
//
// The source is NOT a proof of who the person is: a voice in the room, a phone with the link and a Telegram chat
// are only different doors. The later checks (risk levels, confirmation) use the door as one of their inputs.

use std::cell::Cell;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    // the microphone: the wake word and the recognizer (anyone who can speak near the computer)
    Voice,
    // the Jarvis window of this computer (the local page server)
    Window,
    // the phone remote page (the secret of its link)
    Phone,
    // the Telegram bot (the paired account)
    Telegram,
    // the console of the program (standard input)
    Console,
    // the local WebSocket of the program (another local program)
    Ipc,
    // a script that asked for a phrase to be run and no command was running (a timer, a scenario)
    Script,
    // the program itself (the search after the AI failed)
    Internal,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Voice => "voice",
            Source::Window => "window",
            Source::Phone => "phone",
            Source::Telegram => "telegram",
            Source::Console => "console",
            Source::Ipc => "ipc",
            Source::Script => "script",
            Source::Internal => "internal",
        }
    }

    // a door that is open to somebody who is not at the computer
    pub fn is_remote(self) -> bool {
        matches!(self, Source::Phone | Source::Telegram)
    }
}

// a phrase and its door
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Incoming {
    pub text: String,
    pub source: Source,
    // a script asked for it (jarvis.app.run_text): the door is the one of the command that was running, so a script
    // can never lift a phrase to a more trusted door
    pub via_script: bool,
}

impl Incoming {
    pub fn new(source: Source, text: &str) -> Incoming {
        Incoming { text: clean(text), source, via_script: false }
    }
}

// the text of a request: control characters (the old in-band mark of internal phrases was one of them) become spaces
pub fn clean(text: &str) -> String {
    let spaced: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    spaced.trim().to_string()
}

thread_local! {
    // the door of the command that runs on this thread now (Lua scripts run on the thread of the command)
    static CURRENT: Cell<Option<Source>> = Cell::new(None);
}

pub struct Entered(Option<Source>);

impl Drop for Entered {
    fn drop(&mut self) {
        CURRENT.with(|c| c.set(self.0));
    }
}

// from here until the guard is dropped the commands of this thread come through `source`
pub fn enter(source: Source) -> Entered {
    Entered(CURRENT.with(|c| c.replace(Some(source))))
}

pub fn run_as<R>(source: Source, f: impl FnOnce() -> R) -> R {
    let _g = enter(source);
    f()
}

// the door of the command that runs now; None outside of a command (a test, the command line tool)
pub fn current() -> Option<Source> {
    CURRENT.with(|c| c.get())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_control_character_is_not_a_mark_and_the_text_is_cleaned() {
        assert_eq!(clean("\u{1}найди в интернете кошек"), "найди в интернете кошек");
        assert_eq!(clean("  привет\r\nмир\t"), "привет  мир");
        let i = Incoming::new(Source::Phone, "\u{1}выключи компьютер");
        assert_eq!((i.source, i.text.as_str(), i.via_script), (Source::Phone, "выключи компьютер", false));
    }

    #[test]
    fn the_door_of_a_running_command_is_kept_per_thread_and_restored() {
        assert_eq!(current(), None);
        run_as(Source::Phone, || {
            assert_eq!(current(), Some(Source::Phone));
            run_as(Source::Window, || assert_eq!(current(), Some(Source::Window)));
            assert_eq!(current(), Some(Source::Phone));
            // another thread does not see (and cannot change) this door
            std::thread::spawn(|| {
                assert_eq!(current(), None);
                run_as(Source::Voice, || assert_eq!(current(), Some(Source::Voice)));
            })
            .join()
            .unwrap();
            assert_eq!(current(), Some(Source::Phone));
        });
        assert_eq!(current(), None);
    }

    #[test]
    fn only_the_phone_and_telegram_are_remote() {
        let remote: Vec<_> = [Source::Voice, Source::Window, Source::Phone, Source::Telegram, Source::Console, Source::Ipc, Source::Script, Source::Internal].into_iter().filter(|s| s.is_remote()).collect();
        assert_eq!(remote, vec![Source::Phone, Source::Telegram]);
    }
}
