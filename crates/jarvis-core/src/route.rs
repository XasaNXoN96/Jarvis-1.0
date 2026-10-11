// Where a phrase goes: to a command of the computer, or to the connected AI as a question.
// Commands come first: an explicit command ("открой ворд", "сколько времени") is never sent to the AI.
// A question that asks for knowledge or creative help ("что такое программа", "объясни, как работает Excel",
// "напиши программу на Python") goes to the AI even when it mentions a program name, so it does not start the program.

use crate::apps;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Command,  // search the commands as usual
    Ai,       // ask the connected AI provider
}

// phrases that begin like a question but are commands / small talk of Jarvis himself
const OWN_COMMANDS: &[&str] = &[
    "что ты", "кто ты", "что умеешь", "что можешь", "как дела", "как ты", "как тебя", "как поживаешь", "как жизнь", "как настроение",
    "расскажи о себе", "расскажи про себя", "расскажи о тебе", "кто такой джарвис", "расскажи анекдот", "расскажи шутку", "расскажи что нибудь",
    "что нового", "расскажи новости", "расскажи погоду", "почему не говоришь", "почему молчишь", "почему не работает", "что не работает",
    "что записано", "что запомнил", "сколько времени", "сколько осталось", "какое число", "какой день", "какая погода", "что в буфере",
    "расскажи про погоду", "расскажи о погоде", "расскажи про время",
];

// beginnings that ask for knowledge, an explanation or a text
const KNOWLEDGE_STARTS: &[&str] = &[
    "что такое", "что значит", "что означает", "что за", "кто такой", "кто такая", "кто такие", "кто такое", "объясни", "объясните",
    "расскажи про", "расскажи о", "расскажи об", "расскажите про", "расскажите о", "расскажи мне про", "расскажи мне о", "расскажи как", "расскажи почему",
    "почему", "зачем", "как работает", "как работают", "как устроен", "как устроена", "как называется", "как называют", "как сделать", "как научиться",
    "как правильно", "как лучше", "чем отличается", "чем отличаются", "в чем разница", "в чём разница", "в чем смысл", "какой смысл",
    "что лучше", "что такое", "помоги", "помогите", "составь", "составьте", "придумай", "придумайте", "посоветуй", "посоветуйте", "сравни",
    "дай определение", "определение", "что думаешь", "как думаешь", "что ты думаешь", "как ты думаешь", "правда ли", "можно ли", "верно ли",
    "в чем суть", "в чём суть", "кто изобрел", "кто придумал", "кто создал", "когда был", "когда появил", "сколько лет", "почему так",
];

// "напиши <something to be generated>": a program, a poem, a letter ... (but not "напиши" as a dictation command)
const WRITE_OBJECTS: &[&str] = &[
    "программ", "код", "скрипт", "функци", "стих", "рассказ", "эссе", "реферат", "письмо", "сочинен", "статью", "историю", "сказк", "алгоритм", "класс", "запрос sql",
    "план", "резюме", "доклад", "презентаци", "текст про", "текст о", "текст на тему", "обзор", "описание", "пост",
];

fn norm(text: &str) -> String {
    let words: Vec<String> = apps::tokens(text);
    words.join(" ")
}

fn starts_with_phrase(norm: &str, phrase: &str) -> bool {
    norm == phrase || norm.starts_with(&format!("{} ", phrase))
}

// the phrase is a request for knowledge / generation: it belongs to the AI, not to the commands
pub fn is_knowledge_question(text: &str) -> bool {
    let n = norm(text);
    if n.is_empty() {
        return false;
    }
    if OWN_COMMANDS.iter().any(|p| starts_with_phrase(&n, p)) {
        return false;
    }
    // explicit searches stay commands: "найди в википедии ...", "найди в интернете ..."
    if KNOWLEDGE_STARTS.iter().any(|p| starts_with_phrase(&n, p)) {
        return true;
    }
    for verb in ["напиши", "напишите", "сгенерируй", "создай текст", "сочини", "сочините"] {
        if starts_with_phrase(&n, verb) {
            let rest = n[verb.len()..].trim_start();
            if WRITE_OBJECTS.iter().any(|o| rest.starts_with(o)) {
                return true;
            }
        }
    }
    false
}

// an open question that matched no command ("а где живут пингвины?"): the AI answers it as well
pub fn is_open_question(text: &str) -> bool {
    if is_knowledge_question(text) {
        return true;
    }
    let trimmed = text.trim();
    let words = apps::tokens(trimmed).len();
    let n = norm(trimmed);
    let interrogative = ["как", "какой", "какая", "какое", "какие", "где", "когда", "сколько", "кто", "что", "чей", "куда", "откуда", "почему", "зачем", "ли"]
        .iter()
        .any(|w| starts_with_phrase(&n, w));
    words >= 3 && (trimmed.ends_with('?') || interrogative)
}

pub fn decide(text: &str, ai_ready: bool) -> Route {
    if ai_ready && is_knowledge_question(text) {
        Route::Ai
    } else {
        Route::Command
    }
}

// the phrase to look up on the Internet when the AI cannot answer: the question without its leading words
pub fn search_query(text: &str) -> String {
    let words = apps::tokens(text);
    let skip = ["что", "такое", "кто", "такой", "такая", "объясни", "расскажи", "про", "о", "об", "мне", "пожалуйста", "значит", "означает"];
    let rest: Vec<String> = words.iter().skip_while(|w| skip.contains(&w.as_str())).cloned().collect();
    if rest.is_empty() { words.join(" ") } else { rest.join(" ") }
}

// The words of the user for the conversation and for the AI: the text as it was typed or said (capitals, punctuation),
// without the wake word in front. `matched` is the lower-cased text the commands were searched with.
pub fn original_without_wake(original: &str, matched: &str) -> String {
    let low = original.to_lowercase();
    if let Some(pos) = low.find(matched) {
        let skip = low[..pos].chars().count();
        let take = matched.chars().count();
        let cut: String = original.chars().skip(skip).take(take).collect();
        if !cut.trim().is_empty() {
            // the closing punctuation the user typed ("?" and so on) stays
            let tail: String = original.chars().skip(skip + take).take_while(|c| matches!(c, '?' | '!' | '.' | '…')).collect();
            return format!("{}{}", cut.trim(), tail);
        }
    }
    matched.to_string()
}

#[cfg(test)]
mod tests;
