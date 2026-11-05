package app.jarvis.remote.phone;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

/**
 * Understands what a person says or types to the phone, with the mistakes of a person. Every word of the phrase is first brought to the nearest
 * KNOWN word (a missing or a wrong letter, two swapped, the other layout); the phrase is then read by those words, while the names, the queries
 * and the texts are always taken from the original words (they are never «corrected»). Pure Java: no phone is needed to test it.
 */
public final class Parser {
    /** Names of the contacts of the phone: the parser only needs to know the names. */
    public interface Contacts {
        List<String> names();
    }

    private static final String[] CHAT_WORDS = {"спроси", "расскажи", "объясни", "ответь", "придумай", "переведи", "посчитай", "подскажи", "сочини"};

    private static final String[] KEYWORDS = {
        "открой", "открыть", "откройте", "запустите", "включите", "запусти", "запустить", "зайди", "перейди", "покажи", "включи", "выключи", "поставь", "останови", "пауза", "продолжи", "играй",
        "следующий", "следующую", "предыдущий", "предыдущую", "дальше", "назад", "громче", "тише", "позвони", "набери", "отправь", "напиши", "пиши", "скинь", "пошли",
        "передай", "сообщи", "найди", "поищи", "ищи", "загугли", "запомни", "забудь", "заблокируй", "сделай", "скриншот", "отмени", "погаси",
        "музыку", "музыка", "песню", "трек", "плейлист", "звук", "громкость", "фонарик", "фонарь", "таймер", "будильник", "минут", "минуты", "минуту", "секунд",
        "секунды", "секунду", "часов", "часа", "час", "сообщение", "смс", "экран", "экрана", "телефон", "телефона", "блокировку", "время", "времени", "дату",
        "число", "заряд", "батарею", "погоду", "погода", "файл", "файлы", "документ", "фото", "видео", "настройки", "вайфай", "блютуз", "ютуб", "ютубе",
        "гугл", "телеграм", "ватсап", "приложение", "сайт", "домой", "помощь", "умеешь", "помнишь", "заметки", "компьютер", "компьютера", "пульт", "утра", "вечера",
        "напомни", "напомнить", "напоминание", "напоминай", "через", "снимок", "процентов", "процента", "процент", "полчаса", "который", "сколько", "какое", "какой"
    };

    // the numbers said in words
    private static final String[] ONES = {"ноль", "один", "одну", "два", "две", "три", "четыре", "пять", "шесть", "семь", "восемь", "девять", "десять",
        "одиннадцать", "двенадцать", "тринадцать", "четырнадцать", "пятнадцать", "шестнадцать", "семнадцать", "восемнадцать", "девятнадцать", "двадцать"};
    private static final int[] ONES_V = {0, 1, 1, 2, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20};

    private Parser() { }

    // the words of the phrase twice: the normal form (for reading) and as it was written (for the text of a message, which is never changed)
    static void tokens(String text, List<String> norm, List<String> raw) {
        if (text == null) return;
        for (String chunk : text.trim().split("\\s+")) {
            String n = Norm.normalize(chunk).replace(" ", "");
            if (n.isEmpty()) continue;
            norm.add(n);
            raw.add(chunk);
        }
    }

    // the keyword that the word was meant to be, or the word itself (names and queries are never changed: only the skeleton uses this)
    private static String canon(String w) {
        // the polite forms and the forms of the infinitive are the same command
        switch (w) {
            case "откройте": return "открой";
            case "запустите": return "запусти";
            case "включите": return "включи";
            case "отправьте": case "отправить": return "отправь";
            case "напишите": case "написать": return "напиши";
            case "позвоните": return "позвони";
            default: break;
        }
        for (String k : KEYWORDS) if (k.equals(w)) return k;
        if (w.length() >= 4) {
            String near = Norm.nearWord(w, KEYWORDS);
            if (near != null) return near;
        }
        return w;
    }

    private static boolean contains(String[] arr, String w) {
        for (String a : arr) if (a.equals(w)) return true;
        return false;
    }

    private static boolean has(List<String> c, String... any) {
        for (String w : c) for (String a : any) if (w.equals(a)) return true;
        return false;
    }

    private static int index(List<String> c, String... any) {
        for (int i = 0; i < c.size(); i++) for (String a : any) if (c.get(i).equals(a)) return i;
        return -1;
    }

    private static String join(List<String> o, int from) {
        StringBuilder b = new StringBuilder();
        for (int i = from; i < o.size(); i++) {
            if (b.length() > 0) b.append(' ');
            b.append(o.get(i));
        }
        return b.toString();
    }

    // a number in the words from `from` on: digits or a word of the table; -1 when there is none
    static int number(List<String> o, int from) {
        for (int i = from; i < o.size(); i++) {
            String w = o.get(i);
            if (w.matches("\\d{1,4}")) return Integer.parseInt(w);
            for (int k = 0; k < ONES.length; k++) if (w.equals(ONES[k])) return ONES_V[k];
            int bestK = -1, bestD = 2;
            for (int k = 0; k < ONES.length; k++) {
                if (w.length() >= 5 && ONES[k].length() >= 5) {
                    int d = Norm.typo(w, ONES[k]);
                    if (d < bestD) {
                        bestD = d;
                        bestK = k;
                    }
                }
            }
            if (bestK >= 0) return ONES_V[bestK];
            if (w.equals("тридцать") || w.equals("тридцати")) return 30;
            if (w.equals("сорок")) return 40;
            if (w.equals("пятьдесят") || w.equals("пятидесят")) return 50;
            if (w.equals("шестьдесят")) return 60;
            if (w.equals("девяносто")) return 90;
        }
        return -1;
    }

    // a reminder: the time («через N минут/часов/секунд», «через полчаса», «в 18:30», «в 7 вечера») and the text of it (the rest of the words as they were written)
    static Cmd remind(List<String> o, List<String> c, List<String> raw) {
        boolean[] used = new boolean[o.size()];
        for (int i = 0; i < c.size(); i++) if (c.get(i).equals("напомни") || c.get(i).equals("напомнить") || c.get(i).equals("напоминание") || c.get(i).equals("напоминай")) used[i] = true;
        int seconds = 0, clock = -1;
        int th = index(c, "через");
        if (th >= 0) {
            used[th] = true;
            int u = -1;
            for (int i = th + 1; i < c.size() && u < 0; i++) {
                String w = c.get(i);
                if (w.equals("полчаса") || w.equals("минут") || w.equals("минуты") || w.equals("минуту") || w.equals("секунд") || w.equals("секунды") || w.equals("секунду") || w.equals("часов") || w.equals("часа") || w.equals("час")) u = i;
            }
            if (u < 0) return Cmd.ask("Через сколько напомнить? Например: «напомни через 10 минут позвонить маме».");
            String unit = c.get(u);
            for (int i = th; i <= u; i++) used[i] = true;
            if (unit.equals("полчаса")) seconds = 1800;
            else {
                int n = number(new ArrayList<>(o.subList(th + 1, u)), 0);
                if (n <= 0) n = unit.equals("час") ? 1 : -1;
                if (n <= 0) return Cmd.ask("Через сколько напомнить? Например: «напомни через 10 минут позвонить маме».");
                seconds = unit.startsWith("секунд") ? n : unit.startsWith("час") ? n * 3600 : n * 60;
            }
        } else {
            for (int k = 0; k + 1 < o.size() && clock < 0; k++) {
                if (!o.get(k).equals("в")) continue;
                java.util.regex.Matcher mt = java.util.regex.Pattern.compile("(\\d{1,2})(?:[:.](\\d{2}))?").matcher(raw.get(k + 1));
                if (!mt.matches()) continue;
                int h = Integer.parseInt(mt.group(1)), m = 0, last = k + 1;
                if (mt.group(2) != null) m = Integer.parseInt(mt.group(2));
                else if (k + 2 < o.size() && raw.get(k + 2).matches("\\d{2}")) {
                    m = Integer.parseInt(raw.get(k + 2));
                    last = k + 2;
                }
                if (last + 1 < o.size()) {
                    String d = o.get(last + 1);
                    if (d.equals("вечера") || d.equals("дня")) { if (h < 12) h += 12; last++; }
                    else if (d.equals("утра") || d.equals("ночи")) last++;
                }
                if (h > 23 || m > 59) continue;
                clock = h * 60 + m;
                for (int i = k; i <= last; i++) used[i] = true;
            }
            if (clock < 0) return Cmd.ask("Когда напомнить? Например: «напомни через 10 минут позвонить маме» или «напомни в 18:30 купить хлеб».");
        }
        StringBuilder text = new StringBuilder();
        for (int i = 0; i < raw.size(); i++) {
            if (used[i]) continue;
            String w = o.get(i);
            if (text.length() == 0 && (w.equals("мне") || w.equals("что") || w.equals("о") || w.equals("про") || w.equals("чтобы") || w.equals("пожалуйста") || w.equals("надо") || w.equals("нужно"))) continue;
            if (text.length() > 0) text.append(' ');
            text.append(raw.get(i));
        }
        String t = text.toString().trim();
        if (t.isEmpty()) t = "Напоминание";
        return new Cmd(Cmd.REMIND, t, "", "", clock >= 0 ? -1 : seconds, clock >= 0 ? clock : 0, "");
    }

    /** The phrase as it was said or typed -> what to do. Never null. */
    public static Cmd parse(String text, Contacts contacts) {
        List<String> o = new ArrayList<>();
        List<String> raw = new ArrayList<>();
        tokens(text, o, raw);
        if (!o.isEmpty() && (o.get(0).equals("джарвис") || o.get(0).equals("jarvis") || (o.get(0).length() >= 6 && Norm.nearWord(o.get(0), "джарвис") != null))) {
            o.remove(0);
            raw.remove(0);
        }
        if (o.isEmpty()) return Cmd.of(Cmd.UNKNOWN);
        List<String> names = contacts == null ? new ArrayList<String>() : contacts.names();
        List<String> c = new ArrayList<>();
        for (String w : o) {
            // a word that is (a slip of) a name of a contact is a name, not a command («Тиме» is not «тише»)
            boolean isName = false;
            for (String n : names) {
                for (String part : Norm.words(n)) {
                    if (Norm.sameStem(part, w) || (w.length() >= 4 && Norm.similarity(part, w) >= 0.75)) isName = true;
                }
            }
            c.add(isName && !contains(KEYWORDS, w) ? w : canon(w));
        }

        // --- «на компьютере» / the remote of the computer
        if ((has(c, "компьютер", "компьютера") && (has(c, "пульт", "открой", "запусти", "перейди", "зайди", "покажи"))) || (has(c, "пульт") && has(c, "компьютер", "компьютера"))) {
            return Cmd.of(Cmd.PC_MODE);
        }
        // --- help
        if (has(c, "помощь") || (has(c, "умеешь") && (has(c, "что") || o.size() <= 3))) return Cmd.of(Cmd.HELP);

        // --- a reminder: «напомни через 10 минут позвонить маме», «напомни в 18:30 купить хлеб» (before the call: «позвонить» is a word of the text here)
        if (has(c, "напомни", "напомнить", "напоминание", "напоминай")) return remind(o, c, raw);

        // --- a call / a message
        if (has(c, "позвони", "набери") && o.size() >= 2) {
            int at = Math.max(index(c, "позвони"), index(c, "набери"));
            return Cmd.of(Cmd.CALL, stripPossessive(join(o, at + 1)));
        }
        Cmd msg = message(o, c, raw, contacts);
        if (msg != null) return msg;

        // --- lock / screenshot / home / back
        if (has(c, "заблокируй") || (has(c, "блокировку") && has(c, "включи", "поставь", "сделай")) || (has(c, "выключи") && has(c, "экран", "экрана")) ) {
            return Cmd.of(Cmd.LOCK);
        }
        if (has(c, "скриншот") || (has(c, "снимок") && has(c, "экран", "экрана"))) return Cmd.of(Cmd.SCREENSHOT);
        if (c.size() <= 4 && (has(c, "домой") || (index(c, "на") >= 0 && has(c, "главный")))) return Cmd.of(Cmd.HOME);
        if (c.size() <= 2 && has(c, "назад")) return Cmd.of(Cmd.BACK);

        // --- the torch
        if (has(c, "фонарик", "фонарь")) {
            boolean off = has(c, "выключи", "погаси", "отмени", "останови");
            return Cmd.of(off ? Cmd.TORCH_OFF : Cmd.TORCH_ON);
        }
        // --- timer / alarm
        if (has(c, "таймер")) {
            int at = Math.max(0, index(c, "таймер"));
            int n = number(o, at + 1);
            if (has(c, "отмени", "останови", "выключи")) return Cmd.of(Cmd.TIMER); // n = 0: cancel (Actions says it cannot cancel, it opens the clock)
            int seconds;
            if (has(c, "полчаса")) seconds = 30 * 60;
            else if (n <= 0) return Cmd.ask("На сколько поставить таймер? Например: «таймер на 5 минут».");
            else if (has(c, "секунд", "секунды", "секунду")) seconds = n;
            else if (has(c, "часов", "часа", "час")) seconds = n * 3600;
            else seconds = n * 60;
            return new Cmd(Cmd.TIMER, "", "", "", seconds, 0, "");
        }
        if (has(c, "будильник")) {
            // «на 7 30», «на 7 утра», «на 19:45», «на 7.15»
            int h = -1, m = 0;
            java.util.regex.Matcher mt = java.util.regex.Pattern.compile("(\\d{1,2})\\s*[:.\\s]\\s*(\\d{2})(?!\\d)").matcher(text);
            if (mt.find()) {
                h = Integer.parseInt(mt.group(1));
                m = Integer.parseInt(mt.group(2));
            } else {
                java.util.regex.Matcher m1 = java.util.regex.Pattern.compile("(\\d{1,2})(?!\\d)").matcher(text);
                if (m1.find()) h = Integer.parseInt(m1.group(1));
            }
            if (h < 0) {
                int nn = number(o, Math.max(0, index(c, "будильник")));
                if (nn >= 0) h = nn;
            }
            if (h < 0 || h > 23 || m > 59) return Cmd.ask("На какое время поставить будильник? Например: «будильник на 7 30».");
            if (has(c, "вечера") && h < 12) h += 12;
            return new Cmd(Cmd.ALARM, "", "", "", h, m, "");
        }

        // --- the volume / the music
        if (has(c, "громче")) return Cmd.of(Cmd.VOL_UP);
        if (has(c, "тише")) return Cmd.of(Cmd.VOL_DOWN);
        if (has(c, "звук", "громкость")) {
            int n = number(o, 0);
            boolean off = has(c, "выключи", "убери", "отключи", "без");
            if (has(c, "без")) return Cmd.of(Cmd.MUTE);
            if (off && n < 0) return Cmd.of(Cmd.MUTE);
            if (has(c, "включи") && n < 0) return Cmd.of(Cmd.UNMUTE);
            if (n >= 0 && n <= 100) return new Cmd(Cmd.VOL_SET, "", "", "", n, 0, "");
            if (has(c, "увеличь", "добавь")) return Cmd.of(Cmd.VOL_UP);
        }
        boolean musicWord = has(c, "музыку", "музыка", "песню", "трек", "плейлист");
        if (has(c, "следующий", "следующую", "дальше")) return Cmd.of(Cmd.MUSIC_NEXT);
        if (has(c, "предыдущий", "предыдущую") || (has(c, "назад") && musicWord)) return Cmd.of(Cmd.MUSIC_PREV);
        if (has(c, "пауза") || (has(c, "останови", "выключи") && musicWord) || (has(c, "поставь") && has(c, "паузу", "пауза"))) return Cmd.of(Cmd.MUSIC_PAUSE);
        if (musicWord && has(c, "включи", "поставь", "играй", "запусти", "продолжи", "давай")) return Cmd.of(Cmd.MUSIC_PLAY);
        if (has(c, "продолжи", "играй") && o.size() <= 2) return Cmd.of(Cmd.MUSIC_PLAY);

        // --- time / date / battery / weather
        if ((has(c, "время", "времени") && (has(c, "сколько", "который", "скажи") || o.size() <= 2)) || (has(c, "час") && has(c, "который"))) return Cmd.of(Cmd.TIME);
        if (has(c, "дату", "число") || (has(c, "какой") && index(o, "день") >= 0)) return Cmd.of(Cmd.DATE);
        if (has(c, "заряд", "батарею")) return Cmd.of(Cmd.BATTERY);
        if (has(c, "погоду", "погода")) {
            int at = index(c, "погоду", "погода");
            String city = "";
            int inAt = index(o, "в", "для", "по");
            if (inAt >= 0 && inAt + 1 < o.size()) city = join(o, inAt + 1);
            return Cmd.of(Cmd.WEATHER, city);
        }

        // --- notes
        if (has(c, "запомни")) {
            int at = index(c, "запомни");
            String note = join(o, at + 1);
            if (note.startsWith("что ")) note = note.substring(4);
            return note.isEmpty() ? Cmd.ask("Что запомнить? Например: «запомни, что ключи в сумке».") : Cmd.of(Cmd.REMEMBER, note);
        }
        if (has(c, "помнишь") || (has(c, "заметки")) || (has(c, "что") && index(o, "запомнить") >= 0) || (has(c, "что") && has(c, "запоминал"))) return Cmd.of(Cmd.RECALL);
        if (has(c, "забудь")) return Cmd.of(Cmd.FORGET);

        // --- search: files, YouTube, the web
        if (has(c, "найди", "поищи", "ищи", "загугли")) {
            int at = Math.max(Math.max(index(c, "найди"), index(c, "поищи")), Math.max(index(c, "ищи"), index(c, "загугли")));
            List<String> rest = new ArrayList<>(o.subList(at + 1, o.size()));
            List<String> rc = new ArrayList<>(c.subList(at + 1, c.size()));
            if (rest.isEmpty()) return Cmd.ask("Что искать? Например: «найди рецепт борща».");
            if (!rc.isEmpty() && (rc.get(0).equals("файл") || rc.get(0).equals("файлы") || rc.get(0).equals("документ") || rc.get(0).equals("фото") || rc.get(0).equals("видео"))) {
                String q = join(rest, 1);
                return new Cmd(Cmd.FIND_FILE, q, rc.get(0), "", 0, 0, "");
            }
            int yt = index(rc, "ютуб", "ютубе");
            if (yt >= 0) {
                rest.remove(yt);
                if (!rest.isEmpty() && (rest.get(0).equals("в") || rest.get(0).equals("на"))) rest.remove(0);
                String q = join(rest, 0);
                return Cmd.of(Cmd.YOUTUBE, stripSearchFiller(q));
            }
            return Cmd.of(Cmd.WEB_SEARCH, stripSearchFiller(join(rest, 0)));
        }

        // --- open an app / a site / YouTube
        if (has(c, "открой", "открыть", "запусти", "запустить", "зайди", "перейди", "покажи", "включи")) {
            int at = 0;
            for (int i = 0; i < c.size(); i++) {
                if (c.get(i).equals("открой") || c.get(i).equals("открыть") || c.get(i).equals("запусти") || c.get(i).equals("запустить") || c.get(i).equals("зайди")
                    || c.get(i).equals("перейди") || c.get(i).equals("покажи") || c.get(i).equals("включи")) {
                    at = i;
                    break;
                }
            }
            List<String> rest = new ArrayList<>(o.subList(at + 1, o.size()));
            List<String> rc = new ArrayList<>(c.subList(at + 1, c.size()));
            // «открой ютуб и найди котиков»
            int search = index(rc, "найди", "поищи", "ищи");
            if (search > 0 && index(rc, "ютуб", "ютубе") >= 0) {
                return Cmd.of(Cmd.YOUTUBE, stripSearchFiller(join(rest, search + 1)));
            }
            int skipped = 0;
            while (!rc.isEmpty() && (rc.get(0).equals("на") || rc.get(0).equals("в") || rc.get(0).equals("приложение") || rc.get(0).equals("сайт") || rc.get(0).equals("мне") || rc.get(0).equals("пожалуйста"))) {
                rest.remove(0);
                rc.remove(0);
                skipped++;
            }
            if (rest.isEmpty()) return Cmd.ask("Что открыть? Например: «открой ютуб».");
            if (rc.get(0).equals("настройки")) {
                String which = rc.size() > 1 ? rc.get(1) : "";
                return Cmd.of(Cmd.SETTINGS, which.equals("вайфай") ? "wifi" : which.equals("блютуз") ? "bluetooth" : "");
            }
            if (rc.get(0).equals("вайфай") || rc.get(0).equals("блютуз")) return Cmd.of(Cmd.SETTINGS, rc.get(0).equals("вайфай") ? "wifi" : "bluetooth");
            if (rc.get(0).equals("ютуб") || rc.get(0).equals("ютубе")) return Cmd.of(Cmd.YOUTUBE, "");
            String site = site(rc.get(0));
            if (site != null && rc.size() == 1) return Cmd.of(Cmd.OPEN_SITE, site);
            String name = join(rest, 0);
            String rawName = join(raw, at + 1 + skipped).toLowerCase().replaceAll("^https?://", "");
            if (rawName.matches("[a-z0-9.-]+\\.[a-z]{2,}(/\\S*)?")) return Cmd.of(Cmd.OPEN_SITE, "https://" + rawName);
            return Cmd.of(Cmd.OPEN_APP, name);
        }
        // --- «спроси / расскажи / объясни …»: the question goes to the AI (when the key is set)
        if (o.size() >= 2 && contains(CHAT_WORDS, o.get(0))) return Cmd.of(Cmd.CHAT, join(raw, 1));
        return Cmd.of(Cmd.UNKNOWN);
    }

    // «открой гугл»: the sites that people name
    private static String site(String word) {
        switch (word) {
            case "гугл": return "https://www.google.com";
            case "википедия": return "https://ru.wikipedia.org";
            case "карты": return "https://maps.google.com";
            default: return null;
        }
    }

    // «позвони моей маме» -> «маме»
    private static String stripPossessive(String s) {
        String[] skip = {"моей", "моему", "моя", "мой", "моим", "мою", "своей", "своему", "по", "номеру"};
        List<String> w = Norm.words(s);
        while (!w.isEmpty() && Arrays.asList(skip).contains(w.get(0))) w.remove(0);
        return join(w, 0);
    }

    // «найди информацию про X» / «найди в интернете X» -> X
    private static String stripSearchFiller(String q) {
        List<String> w = Norm.words(q);
        String[] skip = {"в", "на", "интернете", "сети", "информацию", "информация", "инфу", "про", "о", "об", "обо", "мне", "пожалуйста"};
        while (!w.isEmpty() && Arrays.asList(skip).contains(w.get(0))) w.remove(0);
        return join(w, 0);
    }

    private static final String[] TELEGRAM = {"телеграм", "телеграмм", "telegram", "тг"};
    private static final String[] WHATSAPP = {"ватсап", "вацап", "ватцап", "вотсап", "whatsapp", "ватсапп"};

    private static String channelOf(String w) {
        for (String t : TELEGRAM) if (t.equals(w)) return "telegram";
        for (String t : WHATSAPP) if (t.equals(w)) return "whatsapp";
        if (w.length() >= 5) {
            if (Norm.nearWord(w, TELEGRAM) != null) return "telegram";
            if (Norm.nearWord(w, WHATSAPP) != null) return "whatsapp";
        }
        return "";
    }

    private static boolean isPrep(String w) {
        return w.equals("в") || w.equals("по") || w.equals("на") || w.equals("через") || w.equals("via") || w.equals("on") || (w.length() >= 4 && Norm.nearWord(w, "через") != null);
    }

    // «отправь маме через Telegram привет», «отрпавь мамэ чрез тлеграм превет», «смс жене еду домой»: the name must be a contact of the phone
    private static Cmd message(List<String> o, List<String> c, List<String> raw, Contacts contacts) {
        int v = -1;
        for (int i = 0; i < Math.min(c.size(), 2); i++) {
            String w = c.get(i);
            if (w.equals("отправь") || w.equals("напиши") || w.equals("пиши") || w.equals("скинь") || w.equals("пошли") || w.equals("передай") || w.equals("сообщи") || w.equals("смс")) {
                v = i;
                break;
            }
        }
        if (v < 0) return null;
        boolean smsWord = c.get(v).equals("смс") || has(c, "смс");
        int i = v + 1;
        String channel = smsWord ? "sms" : "";
        boolean messageWord = smsWord;
        while (i < o.size()) {
            String w = c.get(i);
            String ow = o.get(i);
            if (w.equals("сообщение") || w.equals("смс") || ow.equals("мне") || ow.equals("моей") || ow.equals("моему") || ow.equals("моя") || ow.equals("мой") || ow.equals("моим")
                || ow.equals("мою") || ow.equals("пожалуйста") || ow.equals("ей") || ow.equals("ему") || ow.equals("a") || ow.equals("to")) {
                if (w.equals("сообщение")) messageWord = true;
                i++;
            } else if (isPrep(ow) && i + 1 < o.size() && !channelOf(o.get(i + 1)).isEmpty()) {
                channel = channelOf(o.get(i + 1));
                i += 2;
            } else if (!channelOf(ow).isEmpty()) {
                channel = channelOf(ow);
                i++;
            } else {
                break;
            }
        }
        if (i >= o.size()) return messageWord || !channel.isEmpty() ? Cmd.ask("Кому и что написать? Например: «отправь маме через Telegram привет».") : null;
        List<String> names = contacts == null ? new ArrayList<String>() : contacts.names();
        // the longest run of 1..3 words that is a contact
        for (int k = Math.min(3, o.size() - i); k >= 1; k--) {
            String who = join(o.subList(i, i + k), 0);
            boolean skip = false;
            for (int t = i; t < i + k; t++) if (!channelOf(o.get(t)).isEmpty() || isPrep(o.get(t)) || o.get(t).equals("что") || o.get(t).equals("текст")) skip = true;
            if (skip) continue;
            String found = findName(who, names);
            if (found == null) continue;
            int j = i + k;
            if (j < o.size()) {
                if (isPrep(o.get(j)) && j + 1 < o.size() && !channelOf(o.get(j + 1)).isEmpty()) {
                    channel = channelOf(o.get(j + 1));
                    j += 2;
                } else if (!channelOf(o.get(j)).isEmpty()) {
                    channel = channelOf(o.get(j));
                    j++;
                }
            }
            return finishMessage(found, channel, raw, j);
        }
        // not a contact: a clear message anyway
        if (messageWord || !channel.isEmpty()) {
            String first = o.get(i);
            return Cmd.ask("Не нашёл в контактах телефона «" + first + "». Проверьте, что контакт сохранён, и что Джарвису разрешён доступ к контактам (в настройках приложения).");
        }
        return null;
    }

    private static Cmd finishMessage(String who, String channel, List<String> raw, int j) {
        String body = join(raw, j).trim();
        // «что …» / «текст …» only introduce the text
        String low = body.toLowerCase();
        if (low.startsWith("что ")) body = body.substring(4).trim();
        else if (low.startsWith("текст ") || low.startsWith("текст:")) body = body.substring(6).trim();
        while (!body.isEmpty() && (body.charAt(0) == ':' || body.charAt(0) == ',' || body.charAt(0) == '-' || body.charAt(0) == '—')) body = body.substring(1).trim();
        if (body.isEmpty()) return Cmd.ask("Что написать «" + who + "»? Скажите, например: «отправь " + who + " привет, как дела».");
        return new Cmd(Cmd.MESSAGE, who, body, channel, 0, 0, "");
    }

    /** the name of a contact that the spoken name stands for (the same word with another ending, or a slip), or null */
    static String findName(String said, List<String> names) {
        String sn = Norm.normalize(said);
        for (String n : names) if (Norm.normalize(n).equals(sn)) return n;
        List<String> sw = Norm.words(said);
        String bestStem = null;
        int stems = 0;
        for (String n : names) {
            List<String> nw = Norm.words(n);
            boolean ok = false;
            if (nw.size() == sw.size()) {
                ok = true;
                for (int i = 0; i < nw.size(); i++) if (!Norm.sameStem(nw.get(i), sw.get(i))) ok = false;
            } else if (sw.size() == 1 && !nw.isEmpty() && Norm.sameStem(nw.get(0), sw.get(0))) {
                ok = true;
            }
            if (ok) {
                bestStem = n;
                stems++;
            }
        }
        if (stems >= 1) return bestStem;
        // a slip in the name
        String best = null;
        double bestS = 0;
        for (String n : names) {
            double s = Norm.similarity(sn, n);
            for (String part : Norm.words(n)) s = Math.max(s, Norm.similarity(sn, part));
            if (s > bestS) {
                bestS = s;
                best = n;
            }
        }
        return bestS >= 0.72 ? best : null;
    }
}
