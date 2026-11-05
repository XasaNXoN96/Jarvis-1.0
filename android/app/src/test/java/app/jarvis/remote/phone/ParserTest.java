package app.jarvis.remote.phone;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import java.util.Arrays;
import java.util.List;
import org.junit.Test;

public class ParserTest {
    private static final Parser.Contacts C = new Parser.Contacts() {
        @Override public List<String> names() { return Arrays.asList("Мама", "Няшка", "Тима Ашуров", "Папа"); }
    };

    private static Cmd p(String s) { return Parser.parse(s, C); }
    private static void is(String phrase, String kind) { assertEquals(phrase, kind, p(phrase).kind); }

    @Test public void apps_and_sites() {
        assertEquals(Cmd.YOUTUBE, p("открой ютуб").kind);
        assertEquals(Cmd.YOUTUBE, p("откройте ютубб").kind);
        assertEquals("котики", p("открой ютуб и найди котики").a);
        assertEquals("котиков", p("открой ютуб и найди котиков").a);
        assertEquals(Cmd.OPEN_SITE, p("открой гугл").kind);
        assertEquals("https://www.google.com", p("открой гугл").a);
        Cmd app = p("открой ватсап");
        assertEquals(Cmd.OPEN_APP, app.kind);
        assertEquals("ватсап", app.a);
        assertEquals(Cmd.OPEN_APP, p("запусти камеру").kind);
        assertEquals("камеру", p("запусти камеру").a);
        assertEquals(Cmd.SETTINGS, p("открой настройки вайфай").kind);
        assertEquals("wifi", p("открой настройки вайфай").a);
        assertEquals(Cmd.OPEN_SITE, p("открой example.com").kind);
    }

    @Test public void mistakes_are_understood() {
        is("откройй ютуб", Cmd.YOUTUBE);
        is("ткрой ютуб", Cmd.YOUTUBE);
        is("включи музыкю", Cmd.MUSIC_PLAY);
        is("вклчи музыку", Cmd.MUSIC_PLAY);
        is("поставь музыку", Cmd.MUSIC_PLAY);
        is("паууза", Cmd.MUSIC_PAUSE);
        is("пауза", Cmd.MUSIC_PAUSE);
        is("следуюший трек", Cmd.MUSIC_NEXT);
        is("громче", Cmd.VOL_UP);
        is("тшие", Cmd.VOL_DOWN);
        is("выключи звук", Cmd.MUTE);
        is("без звука", Cmd.MUTE);
        is("фанарик включи", Cmd.TORCH_ON);
        is("выключи фонарик", Cmd.TORCH_OFF);
        is("заблокируй телефон", Cmd.LOCK);
        is("заблокеруй телефон", Cmd.LOCK);
        is("сделай скриншот", Cmd.SCREENSHOT);
        is("скринщот", Cmd.SCREENSHOT);
        is("сколько времени", Cmd.TIME);
        is("скока времени", Cmd.TIME);
        is("какое число", Cmd.DATE);
        is("сколько заряда", Cmd.BATTERY);
        is("какая погода", Cmd.WEATHER);
        is("погоду в ташкенте", Cmd.WEATHER);
        is("джарвис включи музыку", Cmd.MUSIC_PLAY);
        is("ghbdtn", Cmd.UNKNOWN);
    }

    @Test public void reminders() {
        Cmd a = p("напомни через 10 минут позвонить маме");
        assertEquals(Cmd.REMIND, a.kind);
        assertEquals(600, a.n);
        assertEquals("позвонить маме", a.a);
        Cmd b = p("Джарвис напомни мне через два часа выключить духовку");
        assertEquals(7200, b.n);
        assertEquals("выключить духовку", b.a);
        assertEquals(1800, p("напомни через полчаса про чай").n);
        assertEquals(3600, p("напомни через час купить хлеб").n);
        Cmd c = p("напомни в 18:30 купить хлеб");
        assertEquals(Cmd.REMIND, c.kind);
        assertEquals(-1, c.n);
        assertEquals(18 * 60 + 30, c.m);
        assertEquals("купить хлеб", c.a);
        assertEquals(19 * 60, p("напомни в 7 вечера принять таблетку").m);
        assertEquals(7 * 60 + 15, p("напомни в 7 15 позвонить").m);
        assertEquals(Cmd.ASK, p("напомни купить хлеб").kind);
        assertEquals(600, p("напомни через 10 минут").n);
        assertEquals("Напоминание", p("напомни через 10 минут").a);
        assertEquals(600, p("напомни чрез 10 минут позвонить маме").n);
    }

    @Test public void timers_and_alarms() {
        Cmd t = p("поставь таймер на 5 минут");
        assertEquals(Cmd.TIMER, t.kind);
        assertEquals(300, t.n);
        assertEquals(90, p("таймер на 90 секунд").n);
        assertEquals(600, p("таймер на десять минут").n);
        assertEquals(7200, p("поставь тайемр на 2 часа").n);
        assertEquals(1800, p("таймер на полчаса").n);
        assertEquals(Cmd.ASK, p("поставь таймер").kind);
        Cmd a = p("поставь будильник на 7 30");
        assertEquals(Cmd.ALARM, a.kind);
        assertEquals(7, a.n);
        assertEquals(30, a.m);
        Cmd b = p("будильник на 19:45");
        assertEquals(19, b.n);
        assertEquals(45, b.m);
        assertEquals(19, p("будильник на 7 вечера").n);
        assertEquals(Cmd.ASK, p("будильник").kind);
    }

    @Test public void volume() {
        Cmd v = p("громкость на 40 процентов");
        assertEquals(Cmd.VOL_SET, v.kind);
        assertEquals(40, v.n);
        assertEquals(Cmd.UNMUTE, p("включи звук").kind);
    }

    @Test public void calls_and_messages() {
        Cmd c = p("позвони маме");
        assertEquals(Cmd.CALL, c.kind);
        assertEquals("маме", c.a);
        assertEquals("маме", p("позвони моей маме").a);

        Cmd m = p("отправь маме через Telegram, привет, как дела?");
        assertEquals(Cmd.MESSAGE, m.kind);
        assertEquals("Мама", m.a);
        assertEquals("telegram", m.channel);
        assertEquals("привет, как дела?", m.b);

        Cmd m2 = p("отрпавь маме чрез тлеграм превет как дела");
        assertEquals(Cmd.MESSAGE, m2.kind);
        assertEquals("Мама", m2.a);
        assertEquals("telegram", m2.channel);
        assertEquals("превет как дела", m2.b);

        Cmd m3 = p("напиши в ватсап Тиме Ашурову что встреча в пять");
        assertEquals("Тима Ашуров", m3.a);
        assertEquals("whatsapp", m3.channel);
        assertEquals("встреча в пять", m3.b);

        Cmd m4 = p("пиши моей няшке через телеграм люблю тебя");
        assertEquals("Няшка", m4.a);
        assertEquals("люблю тебя", m4.b);

        Cmd m5 = p("смс папе буду поздно");
        assertEquals(Cmd.MESSAGE, m5.kind);
        assertEquals("Папа", m5.a);
        assertEquals("sms", m5.channel);
        assertEquals("буду поздно", m5.b);

        Cmd m6 = p("отправь сообщение маме. Привет, как дела?");
        assertEquals("Привет, как дела?", m6.b);
        assertEquals("", m6.channel);

        // no such contact / no text: a question, not a guess
        assertEquals(Cmd.ASK, p("отправь сообщение начальнику привет через телеграм").kind);
        assertTrue(p("отправь сообщение начальнику привет через телеграм").say.contains("начальнику"));
        assertEquals(Cmd.ASK, p("отправь маме через телеграм").kind);
        assertTrue(p("отправь маме через телеграм").say.contains("Что написать"));
        // not messages
        assertEquals(Cmd.UNKNOWN, p("отправь файл на почту").kind);
    }

    @Test public void search() {
        Cmd s = p("найди информацию про зум маркет");
        assertEquals(Cmd.WEB_SEARCH, s.kind);
        assertEquals("зум маркет", s.a);
        assertEquals("рецепт борща", p("поищи в интернете рецепт борща").a);
        assertEquals("борщ", p("найди на ютубе борщ").a);
        assertEquals(Cmd.YOUTUBE, p("найди на ютубе борщ").kind);
        Cmd f = p("найди файл отчёт");
        assertEquals(Cmd.FIND_FILE, f.kind);
        assertEquals("отчет", f.a);
        assertEquals(Cmd.ASK, p("найди").kind);
        assertEquals(Cmd.WEB_SEARCH, p("найди рецепт борща").kind);
    }

    @Test public void notes_and_misc() {
        assertEquals("ключи в сумке", p("запомни что ключи в сумке").a);
        assertEquals(Cmd.REMEMBER, p("запомни ключи в сумке").kind);
        assertEquals(Cmd.RECALL, p("что ты помнишь").kind);
        assertEquals(Cmd.HELP, p("что ты умеешь").kind);
        assertEquals(Cmd.HOME, p("домой").kind);
        assertEquals(Cmd.BACK, p("назад").kind);
        assertEquals(Cmd.PC_MODE, p("открой пульт компьютера").kind);
        assertEquals(Cmd.UNKNOWN, p("").kind);
        assertEquals(Cmd.UNKNOWN, p("абвгд ежзий").kind);
    }

    @Test public void words_and_slips() {
        assertEquals(1, Norm.typo("отрпавь", "отправь"));
        assertEquals("привет", Norm.nearWord("ghbdtn", "привет"));
        assertEquals("отправь", Norm.nearWord("отправл", "отправь"));
        assertEquals(null, Norm.nearWord("открой", "отправь"));
        assertTrue(Norm.sameStem("маме", "мама"));
        assertEquals("мама", Norm.normalize("  Мама!! "));
        assertEquals("елка", Norm.normalize("Ёлка"));
    }
}
