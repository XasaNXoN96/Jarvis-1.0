//! Interrupting Jarvis by voice. While he talks, a short spoken «стоп» / «хватит» / «замолчи» silences him (and an AI answer that is
//! still arriving), without the wake word. It is OFF by default (settings → «Перебивание голосом»), because the microphone hears the
//! speakers too. What protects against hearing himself:
//!   * only a phrase that consists of nothing but stop words (and the name of the assistant / «пожалуйста») counts, at most three words;
//!   * a stop word that Jarvis said himself lately (30 s) never counts (he may say «хватит» in an answer);
//!   * the recogniser gives a result when the phrase has ended, so the cut comes a moment after the word, not in the middle of it.
//! The wake word still interrupts, as before (the main loop).

const STOP: &[&str] = &["стоп", "стой", "хватит", "замолчи", "молчи", "помолчи", "достаточно", "перестань", "прекрати"];
const FILLER: &[&str] = &["пожалуйста", "уже", "все", "ну", "так", "давай"];

fn words(text: &str) -> Vec<String> {
    text.to_lowercase().chars().map(|c| if c == 'ё' { 'е' } else if c.is_alphanumeric() { c } else { ' ' }).collect::<String>().split_whitespace().map(|w| w.to_string()).collect()
}

// `heard`: the phrase of the recogniser; `name`: the name of the assistant (lower case, "" = «джарвис»); `said`: what Jarvis said lately,
// as plain words (tts::recent_said)
pub fn is_stop_phrase(heard: &str, name: &str, said: &[String]) -> bool {
    let name = if name.is_empty() { "джарвис".to_string() } else { words(name).join(" ") };
    let w: Vec<String> = words(heard).into_iter().filter(|x| *x != name && !FILLER.contains(&x.as_str())).collect();
    if w.is_empty() || w.len() > 3 || !w.iter().all(|x| STOP.contains(&x.as_str())) {
        return false;
    }
    // Jarvis said this very word himself: it is his own voice from the speakers
    let own: std::collections::HashSet<&str> = said.iter().flat_map(|t| t.split_whitespace()).collect();
    !w.iter().any(|x| own.contains(x.as_str()))
}

// the main loop asks this for every finished phrase while Jarvis speaks; true = he was silenced
pub fn try_interrupt(heard: &str) -> bool {
    if !crate::tts::is_speaking() || !crate::uistate::settings().barge_in {
        return false;
    }
    if !is_stop_phrase(heard, &crate::uistate::assistant_name(), &crate::tts::recent_said()) {
        return false;
    }
    if let Some(id) = crate::ai::store().active() {
        crate::ai::stop(&id);
    }
    crate::tts::stop();
    log::info!("Barge-in: Jarvis was silenced by «{}»", heard);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_phrase_of_stop_words_silences_him() {
        for t in ["стоп", "Стой!", "хватит", "Джарвис, стоп", "замолчи пожалуйста", "хватит хватит", "ну всё хватит", "помолчи", "стоп стоп стоп"] {
            assert!(is_stop_phrase(t, "", &[]), "{}", t);
        }
        for t in ["", "открой браузер", "стоп машина", "не стоп", "как остановить таймер", "хватит ли мне денег", "джарвис", "стой стой стой стой"] {
            assert!(!is_stop_phrase(t, "", &[]), "{}", t);
        }
    }

    #[test]
    fn the_name_of_the_assistant_is_taken_into_account() {
        assert!(is_stop_phrase("карен стоп", "карен", &[]));
        assert!(!is_stop_phrase("карен стоп", "", &[]), "the default name is Джарвис");
        assert!(is_stop_phrase("джарвис хватит", "", &[]));
    }

    #[test]
    fn a_stop_word_that_jarvis_said_himself_is_never_a_command() {
        let said = vec!["этого хватит на неделю".to_string()];
        assert!(!is_stop_phrase("хватит", "", &said), "his own voice from the speakers");
        assert!(is_stop_phrase("стоп", "", &said), "another stop word still works");
        assert!(is_stop_phrase("хватит", "", &["сегодня солнечно".to_string()]));
    }

    #[test]
    fn nothing_happens_when_he_is_not_speaking() {
        assert!(!try_interrupt("стоп"));
    }
}
