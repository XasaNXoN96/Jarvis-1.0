package app.jarvis.remote.phone;

import android.Manifest;
import android.app.Activity;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.ContentResolver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.SharedPreferences;
import android.content.pm.PackageManager;
import android.content.pm.ResolveInfo;
import android.database.Cursor;
import android.hardware.camera2.CameraCharacteristics;
import android.hardware.camera2.CameraManager;
import android.media.AudioManager;
import android.net.Uri;
import android.os.BatteryManager;
import android.os.Build;
import android.os.SystemClock;
import android.provider.AlarmClock;
import android.provider.ContactsContract;
import android.provider.DocumentsContract;
import android.provider.MediaStore;
import android.provider.Settings;
import android.view.KeyEvent;
import android.accessibilityservice.AccessibilityService;

import org.json.JSONArray;
import org.json.JSONObject;

import java.text.SimpleDateFormat;
import java.util.ArrayList;
import java.util.Date;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;

/**
 * What the phone does when the owner says or types a command (the same idea as on the computer: a phrase with mistakes is understood, risky things are
 * asked about, and what cannot be done is said plainly with the reason and what to switch on). The phrase is read by Parser (pure Java); here are the
 * hands: Android intents, the audio, the contacts, the files, the accessibility service. Call `run` away from the main thread.
 */
public class PhoneBrain {
    private static final String PREFS = "jarvis_phone";
    private final Context ctx;
    private final SharedPreferences prefs;
    private Pending pending;

    private static final class Pending {
        final String kind;   // "message" | "forget" | "suggest"
        final Cmd cmd;
        final String phrase;
        final long at = SystemClock.elapsedRealtime();

        Pending(String kind, Cmd cmd, String phrase) {
            this.kind = kind;
            this.cmd = cmd;
            this.phrase = phrase;
        }
    }

    public PhoneBrain(Context ctx) {
        this.ctx = ctx.getApplicationContext();
        this.prefs = this.ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
    }

    // ------------------------------------------------------------------ the answer
    private static String reply(String text, String speak, String need, String action, JSONArray sections, List<String> notes) {
        try {
            JSONObject o = new JSONObject();
            o.put("text", text);
            o.put("speak", speak == null ? text : speak);
            o.put("need", need == null ? "" : need);
            o.put("action", action == null ? "" : action);
            if (sections != null) o.put("sections", sections);
            if (notes != null && !notes.isEmpty()) {
                JSONArray n = new JSONArray();
                for (String s : notes) n.put(s);
                o.put("notes", n);
            }
            return o.toString();
        } catch (Exception e) {
            return "{\"text\":\"ошибка ответа\",\"speak\":\"\",\"need\":\"\",\"action\":\"\"}";
        }
    }

    private static String say(String text) {
        return reply(text, text, "", "", null, null);
    }

    private static String need(String text, String what) {
        return reply(text, text, what, "", null, null);
    }

    // ------------------------------------------------------------------ the contacts of the phone
    private List<String> contactNames = new ArrayList<>();
    private Map<String, String> contactNumbers = new LinkedHashMap<>();
    private long contactsAt = 0;

    boolean has(String perm) {
        return ctx.checkSelfPermission(perm) == PackageManager.PERMISSION_GRANTED;
    }

    private void loadContacts() {
        if (!has(Manifest.permission.READ_CONTACTS)) {
            contactNames = new ArrayList<>();
            contactNumbers = new LinkedHashMap<>();
            return;
        }
        if (SystemClock.elapsedRealtime() - contactsAt < 60_000 && !contactNames.isEmpty()) return;
        List<String> names = new ArrayList<>();
        Map<String, String> numbers = new LinkedHashMap<>();
        Cursor c = null;
        try {
            c = ctx.getContentResolver().query(ContactsContract.CommonDataKinds.Phone.CONTENT_URI,
                new String[]{ContactsContract.CommonDataKinds.Phone.DISPLAY_NAME, ContactsContract.CommonDataKinds.Phone.NUMBER}, null, null, null);
            while (c != null && c.moveToNext() && names.size() < 5000) {
                String name = c.getString(0);
                String num = c.getString(1);
                if (name == null || num == null || numbers.containsKey(name)) continue;
                names.add(name);
                numbers.put(name, num);
            }
        } catch (RuntimeException ignored) {
        } finally {
            if (c != null) c.close();
        }
        contactNames = names;
        contactNumbers = numbers;
        contactsAt = SystemClock.elapsedRealtime();
    }

    private final Parser.Contacts contacts = new Parser.Contacts() {
        @Override
        public List<String> names() {
            loadContacts();
            return contactNames;
        }
    };

    private static String digits(String number) {
        String d = number.replaceAll("[^0-9]", "");
        if (d.startsWith("00")) d = d.substring(2);
        return d;
    }

    // ------------------------------------------------------------------ the entry
    private static boolean yes(String w) {
        return w.equals("да") || w.equals("ага") || w.equals("угу") || w.equals("конечно") || w.equals("давай") || w.equals("подтверждаю") || w.equals("отправляй") || w.equals("верно") || w.equals("ок") || w.equals("окей");
    }

    private static boolean no(String w) {
        return w.equals("нет") || w.equals("отмена") || w.equals("отмени") || w.equals("стоп") || w.equals("неа") || w.equals("не");
    }

    /** One phrase of the owner -> the answer as JSON (text, speak, need, action, sections, notes). Never throws. */
    public String run(String text) {
        try {
            return run0(text == null ? "" : text.trim());
        } catch (RuntimeException e) {
            return say("Не получилось: " + (e.getMessage() == null ? e.getClass().getSimpleName() : e.getMessage()));
        }
    }

    private String run0(String text) {
        if (text.isEmpty()) return say("Скажите или напишите команду. «Что ты умеешь» покажет примеры.");
        // «да / нет» to an open question (90 seconds)
        if (pending != null && SystemClock.elapsedRealtime() - pending.at < 90_000) {
            String w = Norm.normalize(text);
            Pending p = pending;
            if (yes(w)) {
                pending = null;
                return confirmed(p);
            }
            if (no(w)) {
                pending = null;
                return say("Хорошо, не выполняю.");
            }
            pending = null;   // anything else is a new phrase
        }
        Cmd c = Parser.parse(text, contacts);
        switch (c.kind) {
            case Cmd.ASK: return say(c.say);
            case Cmd.UNKNOWN: return unknown(text);
            case Cmd.OPEN_APP: return openApp(c.a);
            case Cmd.OPEN_SITE: return view(c.a, "Открываю " + WebSearch.hostOf(c.a));
            case Cmd.YOUTUBE: return youtube(c.a);
            case Cmd.WEB_SEARCH: return webSearch(c.a);
            case Cmd.FIND_FILE: return findFile(c.a, c.b);
            case Cmd.MUSIC_PLAY: return musicPlay();
            case Cmd.MUSIC_PAUSE: media(KeyEvent.KEYCODE_MEDIA_PAUSE); return say("Пауза.");
            case Cmd.MUSIC_NEXT: media(KeyEvent.KEYCODE_MEDIA_NEXT); return say("Следующий трек.");
            case Cmd.MUSIC_PREV: media(KeyEvent.KEYCODE_MEDIA_PREVIOUS); return say("Предыдущий трек.");
            case Cmd.VOL_UP: volume(AudioManager.ADJUST_RAISE); return say("Громче.");
            case Cmd.VOL_DOWN: volume(AudioManager.ADJUST_LOWER); return say("Тише.");
            case Cmd.MUTE: volume(AudioManager.ADJUST_MUTE); return say("Звук выключен.");
            case Cmd.UNMUTE: volume(AudioManager.ADJUST_UNMUTE); return say("Звук включён.");
            case Cmd.VOL_SET: return volumeSet(c.n);
            case Cmd.TIMER: return timer(c.n);
            case Cmd.REMIND: return remind(c);
            case Cmd.CHAT: return chat(c.a);
            case Cmd.ALARM: return alarm(c.n, c.m);
            case Cmd.TORCH_ON: return torch(true);
            case Cmd.TORCH_OFF: return torch(false);
            case Cmd.CALL: return call(c.a);
            case Cmd.MESSAGE: return message(c);
            case Cmd.LOCK: return global(c.kind);
            case Cmd.SCREENSHOT: return global(c.kind);
            case Cmd.HOME: return home();
            case Cmd.BACK: return global(c.kind);
            case Cmd.TIME: return say("Сейчас " + new SimpleDateFormat("HH:mm", new Locale("ru")).format(new Date()) + ".");
            case Cmd.DATE: return say("Сегодня " + new SimpleDateFormat("EEEE, d MMMM yyyy", new Locale("ru")).format(new Date()) + ".");
            case Cmd.BATTERY: return battery();
            case Cmd.WEATHER: return weather(c.a);
            case Cmd.REMEMBER: return remember(c.a);
            case Cmd.RECALL: return recall();
            case Cmd.FORGET:
                pending = new Pending("forget", c, text);
                return say("Забыть все заметки? Скажите «да» или «нет».");
            case Cmd.SETTINGS: return settings(c.a);
            case Cmd.PC_MODE: return reply("Открываю пульт компьютера.", "Открываю пульт компьютера.", "", "pc", null, null);
            case Cmd.HELP: return help();
            default: return unknown(text);
        }
    }

    private String confirmed(Pending p) {
        switch (p.kind) {
            case "message": return sendMessage(p.cmd, true);
            case "forget":
                prefs.edit().remove("notes").apply();
                return say("Заметки удалены.");
            case "suggest": return run0(p.phrase);
            default: return say("Хорошо.");
        }
    }

    // ------------------------------------------------------------------ an unknown phrase: the nearest known one is offered
    private static final String[] EXAMPLES = {
        "открой ютуб", "открой ватсап", "открой камеру", "включи музыку", "поставь на паузу", "следующий трек", "громче", "тише", "без звука", "включи фонарик", "выключи фонарик",
        "поставь таймер на 5 минут", "поставь будильник на 7 30", "позвони маме", "отправь маме через телеграм привет", "заблокируй телефон", "сделай скриншот", "который час",
        "какое число", "сколько заряда", "какая погода", "запомни ключи в сумке", "что ты помнишь", "найди информацию про зум маркет", "найди файл отчет", "открой пульт компьютера"
    };

    private String unknown(String text) {
        String best = null;
        double bs = 0;
        for (String e : EXAMPLES) {
            double s = Norm.similarity(text, e);
            if (s > bs) {
                bs = s;
                best = e;
            }
        }
        if (best != null && bs >= 0.62) {
            pending = new Pending("suggest", null, best);
            return say("Не уверен, что понял. Вы имели в виду «" + best + "»? Скажите «да» или «нет».");
        }
        if (!aiKey().isEmpty() && text.trim().length() >= 6) return chat(text);
        return say("Не понял «" + text + "». Скажите «что ты умеешь», чтобы увидеть примеры.");
    }

    private String help() {
        StringBuilder b = new StringBuilder("Я умею (примеры):\n");
        for (String e : EXAMPLES) b.append("• ").append(e).append('\n');
        b.append("\nФразы можно говорить и писать с ошибками. Блокировка, скриншот и кнопка «Отправить» в Telegram и WhatsApp работают после включения «Доступности» (кнопка в настройках приложения).");
        return reply(b.toString(), "Примеры команд показаны на экране.", "", "", null, null);
    }

    // ------------------------------------------------------------------ opening
    private String startView(Intent i) {
        i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        try {
            ctx.startActivity(i);
            return null;
        } catch (RuntimeException e) {
            return "Не получилось открыть: нет подходящего приложения.";
        }
    }

    private String view(String url, String said) {
        String err = startView(new Intent(Intent.ACTION_VIEW, Uri.parse(url)));
        return say(err != null ? err : said);
    }

    private String youtube(String q) {
        if (q.isEmpty()) {
            Intent app = ctx.getPackageManager().getLaunchIntentForPackage("com.google.android.youtube");
            if (app != null) {
                String err = startView(app);
                return say(err != null ? err : "Открываю YouTube.");
            }
            return view("https://www.youtube.com", "Открываю YouTube.");
        }
        return view("https://www.youtube.com/results?search_query=" + WebSearch.enc(q), "Ищу на YouTube: " + q + ".");
    }

    private static final Map<String, String[]> ALIASES = new LinkedHashMap<>();

    static {
        ALIASES.put("ватсап", new String[]{"whatsapp"});
        ALIASES.put("вацап", new String[]{"whatsapp"});
        ALIASES.put("телеграм", new String[]{"telegram"});
        ALIASES.put("инстаграм", new String[]{"instagram"});
        ALIASES.put("камера", new String[]{"camera", "камера"});
        ALIASES.put("камеру", new String[]{"camera", "камера"});
        ALIASES.put("галерея", new String[]{"gallery", "photos", "галерея", "фото"});
        ALIASES.put("галерею", new String[]{"gallery", "photos", "галерея", "фото"});
        ALIASES.put("фото", new String[]{"photos", "gallery", "фото"});
        ALIASES.put("часы", new String[]{"clock", "часы"});
        ALIASES.put("калькулятор", new String[]{"calculator", "калькулятор"});
        ALIASES.put("браузер", new String[]{"chrome", "browser", "браузер"});
        ALIASES.put("хром", new String[]{"chrome"});
        ALIASES.put("карты", new String[]{"maps", "карты"});
        ALIASES.put("почта", new String[]{"gmail", "почта", "mail"});
        ALIASES.put("контакты", new String[]{"contacts", "контакты"});
        ALIASES.put("сообщения", new String[]{"messages", "сообщения"});
        ALIASES.put("музыка", new String[]{"music", "музыка"});
        ALIASES.put("музыку", new String[]{"music", "музыка"});
        ALIASES.put("календарь", new String[]{"calendar", "календарь"});
        ALIASES.put("магазин", new String[]{"play store", "play маркет"});
        ALIASES.put("плей", new String[]{"play store", "play маркет"});
    }

    private String openApp(String name) {
        PackageManager pm = ctx.getPackageManager();
        Intent main = new Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER);
        List<ResolveInfo> apps = pm.queryIntentActivities(main, 0);
        String want = Norm.normalize(name);
        String[] alias = ALIASES.get(want);
        ResolveInfo best = null;
        double bestScore = 0;
        for (ResolveInfo r : apps) {
            String label = Norm.normalize(String.valueOf(r.loadLabel(pm)));
            if (label.isEmpty()) continue;
            double s = 0;
            if (label.equals(want)) s = 1.0;
            else if (label.contains(want) || want.contains(label)) s = 0.9;
            else s = Norm.similarity(label, want);
            if (alias != null) for (String a : alias) {
                String an = Norm.normalize(a);
                if (label.equals(an)) s = Math.max(s, 0.98);
                else if (label.contains(an)) s = Math.max(s, 0.92);
            }
            if (s > bestScore) {
                bestScore = s;
                best = r;
            }
        }
        if (best == null || bestScore < 0.72) {
            return say("Не нашёл приложение «" + name + "» на телефоне.");
        }
        Intent i = pm.getLaunchIntentForPackage(best.activityInfo.packageName);
        if (i == null) return say("Не получилось открыть «" + name + "».");
        String err = startView(i);
        return say(err != null ? err : "Открываю " + best.loadLabel(pm) + ".");
    }

    private String settings(String which) {
        String action = Settings.ACTION_SETTINGS;
        if ("wifi".equals(which)) action = Settings.ACTION_WIFI_SETTINGS;
        else if ("bluetooth".equals(which)) action = Settings.ACTION_BLUETOOTH_SETTINGS;
        String err = startView(new Intent(action));
        return say(err != null ? err : "Открываю настройки.");
    }

    // ------------------------------------------------------------------ the music, the volume
    private AudioManager audio() {
        return (AudioManager) ctx.getSystemService(Context.AUDIO_SERVICE);
    }

    private void media(int code) {
        AudioManager am = audio();
        long t = SystemClock.uptimeMillis();
        am.dispatchMediaKeyEvent(new KeyEvent(t, t, KeyEvent.ACTION_DOWN, code, 0));
        am.dispatchMediaKeyEvent(new KeyEvent(t, t, KeyEvent.ACTION_UP, code, 0));
    }

    private String musicPlay() {
        AudioManager am = audio();
        if (am.isMusicActive()) return say("Музыка уже играет.");
        media(KeyEvent.KEYCODE_MEDIA_PLAY);
        try {
            Thread.sleep(900);
        } catch (InterruptedException ignored) {
        }
        if (!am.isMusicActive()) {
            // no player was waiting for the key: the music application of the phone is opened
            Intent i = Intent.makeMainSelectorActivity(Intent.ACTION_MAIN, Intent.CATEGORY_APP_MUSIC);
            String err = startView(i);
            return say(err != null ? "Не нашёл музыкальное приложение. Скажите «открой ютуб» или «открой музыку»." : "Открываю музыку: нажмите воспроизведение.");
        }
        return say("Включаю музыку.");
    }

    private void volume(int dir) {
        audio().adjustStreamVolume(AudioManager.STREAM_MUSIC, dir, AudioManager.FLAG_SHOW_UI);
    }

    private String volumeSet(int percent) {
        AudioManager am = audio();
        int max = am.getStreamMaxVolume(AudioManager.STREAM_MUSIC);
        int v = Math.max(0, Math.min(max, Math.round(max * percent / 100f)));
        am.setStreamVolume(AudioManager.STREAM_MUSIC, v, AudioManager.FLAG_SHOW_UI);
        return say("Громкость " + percent + " процентов.");
    }

    // ------------------------------------------------------------------ timers, alarms, the torch
    private String timer(int seconds) {
        if (seconds <= 0) {
            String err = startView(new Intent(AlarmClock.ACTION_SHOW_TIMERS));
            return say(err != null ? err : "Открываю таймеры: отмените нужный.");
        }
        Intent i = new Intent(AlarmClock.ACTION_SET_TIMER).putExtra(AlarmClock.EXTRA_LENGTH, seconds).putExtra(AlarmClock.EXTRA_MESSAGE, "Jarvis").putExtra(AlarmClock.EXTRA_SKIP_UI, true);
        String err = startView(i);
        if (err != null) return say("Не получилось поставить таймер: " + err);
        String how = seconds % 3600 == 0 ? (seconds / 3600) + " ч" : seconds % 60 == 0 ? (seconds / 60) + " мин" : seconds + " с";
        return say("Таймер на " + how + " поставлен.");
    }

    // ------------------------------------------------------------------ the AI (the key of the owner is typed on this phone and stays here)
    private final List<String[]> chatHistory = new ArrayList<>();

    private String aiKey() {
        return prefs.getString("ai_key", "");
    }

    public void setAiKey(String key) {
        String k = key == null ? "" : key.trim();
        prefs.edit().putString("ai_key", k.length() > 300 ? "" : k).apply();
        chatHistory.clear();
    }

    private String chat(String question) {
        String key = aiKey();
        if (key.isEmpty()) return say("Чтобы я отвечал как ИИ, введите свой ключ OpenAI в настройках телефона (шестерёнка, поле «Ключ OpenAI»).");
        try {
            String a = AiChat.ask(key, question, chatHistory);
            chatHistory.add(new String[]{question, a});
            while (chatHistory.size() > 6) chatHistory.remove(0);
            return say(a);
        } catch (IllegalStateException e) {
            return say("ИИ не ответил: " + e.getMessage() + ".");
        }
    }

    // a reminder: a notification of the phone at the time (AlarmManager, no foreground service); it is lost when the phone is switched off
    private String remind(Cmd c) {
        if (Build.VERSION.SDK_INT >= 33 && !has("android.permission.POST_NOTIFICATIONS")) {
            return need("Чтобы напоминание появилось, разрешите приложению показывать уведомления.", "notify");
        }
        long at;
        if (c.n >= 0) at = System.currentTimeMillis() + c.n * 1000L;
        else {
            java.util.Calendar cal = java.util.Calendar.getInstance();
            cal.set(java.util.Calendar.HOUR_OF_DAY, c.m / 60);
            cal.set(java.util.Calendar.MINUTE, c.m % 60);
            cal.set(java.util.Calendar.SECOND, 0);
            cal.set(java.util.Calendar.MILLISECOND, 0);
            if (cal.getTimeInMillis() <= System.currentTimeMillis() + 5000) cal.add(java.util.Calendar.DAY_OF_YEAR, 1);
            at = cal.getTimeInMillis();
        }
        try {
            int id = (int) (at / 1000L) ^ c.a.hashCode();
            Intent i = new Intent(ctx, ReminderReceiver.class).putExtra("text", c.a).putExtra("id", id);
            android.app.PendingIntent pi = android.app.PendingIntent.getBroadcast(ctx, id, i, android.app.PendingIntent.FLAG_UPDATE_CURRENT | android.app.PendingIntent.FLAG_IMMUTABLE);
            android.app.AlarmManager am = (android.app.AlarmManager) ctx.getSystemService(Context.ALARM_SERVICE);
            if (Build.VERSION.SDK_INT >= 31 && !am.canScheduleExactAlarms()) am.setAndAllowWhileIdle(android.app.AlarmManager.RTC_WAKEUP, at, pi);
            else am.setExactAndAllowWhileIdle(android.app.AlarmManager.RTC_WAKEUP, at, pi);
        } catch (Exception e) {
            return say("Не получилось поставить напоминание: " + e.getMessage());
        }
        String when = new SimpleDateFormat("HH:mm", new Locale("ru")).format(new Date(at));
        return say("Напомню в " + when + ": " + c.a + ".");
    }

    private String alarm(int h, int m) {
        Intent i = new Intent(AlarmClock.ACTION_SET_ALARM).putExtra(AlarmClock.EXTRA_HOUR, h).putExtra(AlarmClock.EXTRA_MINUTES, m).putExtra(AlarmClock.EXTRA_MESSAGE, "Jarvis").putExtra(AlarmClock.EXTRA_SKIP_UI, true);
        String err = startView(i);
        if (err != null) return say("Не получилось поставить будильник: " + err);
        return say(String.format(Locale.US, "Будильник на %02d:%02d поставлен.", h, m));
    }

    private String torch(boolean on) {
        try {
            CameraManager cm = (CameraManager) ctx.getSystemService(Context.CAMERA_SERVICE);
            for (String id : cm.getCameraIdList()) {
                CameraCharacteristics ch = cm.getCameraCharacteristics(id);
                Boolean flash = ch.get(CameraCharacteristics.FLASH_INFO_AVAILABLE);
                Integer facing = ch.get(CameraCharacteristics.LENS_FACING);
                if (flash != null && flash && facing != null && facing == CameraCharacteristics.LENS_FACING_BACK) {
                    cm.setTorchMode(id, on);
                    return say(on ? "Фонарик включён." : "Фонарик выключен.");
                }
            }
            return say("На этом телефоне нет вспышки для фонарика.");
        } catch (Exception e) {
            return say("Не получилось управлять фонариком: " + e.getMessage());
        }
    }

    // ------------------------------------------------------------------ a call, a message
    private String numberOf(String spoken) {
        loadContacts();
        String name = Parser.findName(spoken, contactNames);
        return name == null ? null : contactNumbers.get(name);
    }

    private String call(String who) {
        if (who.replaceAll("[^0-9+]", "").length() >= 5 && who.matches(".*\\d.*")) {
            return dial(who, who);
        }
        if (!has(Manifest.permission.READ_CONTACTS)) return need("Чтобы звонить по имени, разрешите приложению доступ к контактам.", "contacts");
        String num = numberOf(who);
        if (num == null) return say("Не нашёл в контактах «" + who + "».");
        return dial(num, who);
    }

    private String dial(String number, String label) {
        Uri uri = Uri.parse("tel:" + Uri.encode(number));
        boolean direct = has(Manifest.permission.CALL_PHONE);
        String err = startView(new Intent(direct ? Intent.ACTION_CALL : Intent.ACTION_DIAL, uri));
        if (err != null) return say(err);
        return say(direct ? "Звоню: " + label + "." : "Набираю номер «" + label + "»: нажмите зелёную трубку. (Чтобы звонить сразу, разрешите приложению звонки.)");
    }

    private String message(Cmd c) {
        if (!has(Manifest.permission.READ_CONTACTS)) return need("Чтобы писать по имени, разрешите приложению доступ к контактам.", "contacts");
        if (c.channel.isEmpty()) {
            return say("Через что отправить «" + c.a + "»: Telegram, WhatsApp или СМС? Скажите, например: «отправь " + c.a + " через Telegram " + c.b + "».");
        }
        if (JarvisAccess.enabled() && !c.channel.equals("sms")) {
            pending = new Pending("message", c, "");
            String app = c.channel.equals("whatsapp") ? "WhatsApp" : "Telegram";
            return say("Сообщение для «" + c.a + "» (" + app + "): «" + c.b + "». Отправить? Скажите «да» или «нет».");
        }
        return sendMessage(c, false);
    }

    // the chat is opened with the text; with the accessibility service (and after «да») Jarvis presses «Отправить» itself
    private String sendMessage(Cmd c, boolean press) {
        String num = contactNumbers.get(c.a);
        if (num == null) {
            loadContacts();
            num = contactNumbers.get(c.a);
        }
        if (num == null) return say("Не нашёл номер для «" + c.a + "».");
        String d = digits(num);
        String err;
        boolean textInLink;
        switch (c.channel) {
            case "whatsapp": {
                Intent i = new Intent(Intent.ACTION_VIEW, Uri.parse("https://wa.me/" + d + "?text=" + WebSearch.enc(c.b)));
                if (installed("com.whatsapp")) i.setPackage("com.whatsapp");
                else if (installed("com.whatsapp.w4b")) i.setPackage("com.whatsapp.w4b");
                err = startView(i);
                textInLink = true;
                break;
            }
            case "telegram": {
                copy(c.b);
                Intent i = new Intent(Intent.ACTION_VIEW, Uri.parse("tg://resolve?phone=" + d));
                if (installed("org.telegram.messenger")) i.setPackage("org.telegram.messenger");
                else if (installed("org.telegram.messenger.web")) i.setPackage("org.telegram.messenger.web");
                err = startView(i);
                textInLink = false;
                break;
            }
            default: {
                Intent i = new Intent(Intent.ACTION_SENDTO, Uri.parse("smsto:" + Uri.encode(num))).putExtra("sms_body", c.b);
                err = startView(i);
                textInLink = true;
                break;
            }
        }
        if (err != null) return say("Не получилось открыть чат: " + err);
        String app = c.channel.equals("whatsapp") ? "WhatsApp" : c.channel.equals("telegram") ? "Telegram" : "СМС";
        if (press && JarvisAccess.enabled()) {
            JarvisAccess.autoSend(c.b, !textInLink, null);
            return say("Открыл чат с «" + c.a + "» (" + app + ") и нажимаю «Отправить». Дошло ли сообщение, я проверить не могу.");
        }
        if (c.channel.equals("telegram")) return say("Чат с «" + c.a + "» открыт, текст скопирован: вставьте его и нажмите «Отправить».");
        return say("Чат с «" + c.a + "» (" + app + ") открыт с текстом: нажмите «Отправить».");
    }

    private boolean installed(String pkg) {
        try {
            ctx.getPackageManager().getPackageInfo(pkg, 0);
            return true;
        } catch (PackageManager.NameNotFoundException e) {
            return false;
        }
    }

    private void copy(String text) {
        try {
            ClipboardManager cm = (ClipboardManager) ctx.getSystemService(Context.CLIPBOARD_SERVICE);
            cm.setPrimaryClip(ClipData.newPlainText("Jarvis", text));
        } catch (RuntimeException ignored) {
        }
    }

    // ------------------------------------------------------------------ the accessibility service: lock, screenshot, home, back
    private String global(String kind) {
        if (kind.equals(Cmd.HOME)) return home();
        if (!JarvisAccess.enabled()) {
            return need("Чтобы блокировать телефон, делать скриншот и нажимать «назад» голосом, включите службу «Jarvis» в Специальных возможностях. Нажмите кнопку ниже.", "access");
        }
        if (!JarvisAccess.canLockAndShoot() && !kind.equals(Cmd.BACK)) return say("Блокировка и скриншот голосом работают на Android 9 и новее.");
        int action = kind.equals(Cmd.LOCK) ? 8 /* GLOBAL_ACTION_LOCK_SCREEN */ : kind.equals(Cmd.SCREENSHOT) ? 9 /* GLOBAL_ACTION_TAKE_SCREENSHOT */ : AccessibilityService.GLOBAL_ACTION_BACK;
        boolean ok = JarvisAccess.global(action);
        if (!ok) return say("Не получилось: Android не принял команду.");
        return say(kind.equals(Cmd.LOCK) ? "Блокирую телефон." : kind.equals(Cmd.SCREENSHOT) ? "Делаю скриншот." : "Назад.");
    }

    private String home() {
        if (JarvisAccess.enabled()) {
            JarvisAccess.global(AccessibilityService.GLOBAL_ACTION_HOME);
            return say("Домой.");
        }
        String err = startView(new Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME));
        return say(err != null ? err : "Домой.");
    }

    // ------------------------------------------------------------------ facts
    private String battery() {
        Intent b = ctx.registerReceiver(null, new IntentFilter(Intent.ACTION_BATTERY_CHANGED));
        if (b == null) return say("Не получилось узнать заряд.");
        int level = b.getIntExtra(BatteryManager.EXTRA_LEVEL, -1);
        int scale = b.getIntExtra(BatteryManager.EXTRA_SCALE, 100);
        int status = b.getIntExtra(BatteryManager.EXTRA_STATUS, -1);
        boolean charging = status == BatteryManager.BATTERY_STATUS_CHARGING || status == BatteryManager.BATTERY_STATUS_FULL;
        int pct = scale > 0 ? Math.round(level * 100f / scale) : level;
        return say("Заряд " + pct + " процентов" + (charging ? ", телефон заряжается." : "."));
    }

    private static String wmo(int code) {
        if (code == 0) return "ясно";
        if (code == 1) return "в основном ясно";
        if (code == 2) return "переменная облачность";
        if (code == 3) return "пасмурно";
        if (code == 45 || code == 48) return "туман";
        if (code >= 51 && code <= 57) return "морось";
        if ((code >= 61 && code <= 67) || (code >= 80 && code <= 82)) return "дождь";
        if ((code >= 71 && code <= 77) || code == 85 || code == 86) return "снег";
        if (code >= 95) return "гроза";
        return "без данных";
    }

    private String weather(String cityArg) {
        String city = cityArg == null ? "" : cityArg.trim();
        if (city.isEmpty()) city = prefs.getString("city", "");
        if (city.isEmpty()) return say("Для какого города? Скажите, например: «погода в Ташкенте».");
        try {
            JSONObject geo = new JSONObject(WebSearch.get("https://geocoding-api.open-meteo.com/v1/search?name=" + WebSearch.enc(city) + "&count=1&language=ru&format=json"));
            JSONArray res = geo.optJSONArray("results");
            if (res == null || res.length() == 0) return say("Не нашёл город «" + city + "».");
            JSONObject g = res.getJSONObject(0);
            String place = g.optString("name", city);
            JSONObject f = new JSONObject(WebSearch.get("https://api.open-meteo.com/v1/forecast?latitude=" + g.getDouble("latitude") + "&longitude=" + g.getDouble("longitude")
                + "&current=temperature_2m,weather_code,wind_speed_10m&timezone=auto"));
            JSONObject cur = f.getJSONObject("current");
            prefs.edit().putString("city", city).apply();
            long t = Math.round(cur.getDouble("temperature_2m"));
            return say(place + ": " + t + " градусов, " + wmo(cur.getInt("weather_code")) + ", ветер " + Math.round(cur.optDouble("wind_speed_10m", 0)) + " метров в секунду.");
        } catch (Exception e) {
            return say("Не получилось узнать погоду: " + (e instanceof java.net.UnknownHostException ? "нет соединения с интернетом" : String.valueOf(e.getMessage())));
        }
    }

    // ------------------------------------------------------------------ notes
    private JSONArray notes() {
        try {
            return new JSONArray(prefs.getString("notes", "[]"));
        } catch (Exception e) {
            return new JSONArray();
        }
    }

    private String remember(String note) {
        JSONArray all = notes();
        if (all.length() >= 200) return say("Заметок уже 200: скажите «забудь всё» и начните заново.");
        all.put(note);
        prefs.edit().putString("notes", all.toString()).apply();
        return say("Запомнил: " + note + ".");
    }

    private String recall() {
        JSONArray all = notes();
        if (all.length() == 0) return say("Пока ничего не записано. Скажите «запомни …».");
        StringBuilder b = new StringBuilder("Вы просили запомнить:\n");
        int from = Math.max(0, all.length() - 15);
        for (int i = from; i < all.length(); i++) b.append("• ").append(all.optString(i)).append('\n');
        return reply(b.toString(), "Вы просили запомнить " + all.length() + " " + (all.length() == 1 ? "заметку" : "заметок") + ". Они на экране.", "", "", null, null);
    }

    // ------------------------------------------------------------------ search in the Internet and in the files
    private String webSearch(String q) {
        if (q.isEmpty()) return say("Что искать? Например: «найди рецепт борща».");
        WebSearch.Result r = WebSearch.search(q);
        JSONArray sections = new JSONArray();
        try {
            addSection(sections, "photos", "📷 Фото", r.photos);
            addSection(sections, "videos", "▶ Видео", r.videos);
            addSection(sections, "links", "🔗 Ссылки", r.links);
            if (r.text != null) {
                List<WebSearch.Item> t = new ArrayList<>();
                t.add(r.text);
                addSection(sections, "text", "📝 Текст: " + r.text.title, t);
            }
        } catch (Exception ignored) {
        }
        int found = r.photos.size() + r.videos.size() + r.links.size() + (r.text != null ? 1 : 0);
        String summary = found == 0 ? "По запросу «" + q + "» ничего не нашёл." : "Нашёл по запросу «" + q + "»: фото " + r.photos.size() + ", видео " + r.videos.size() + ", ссылок " + r.links.size() + (r.text != null ? ", и статью" : "") + ".";
        String speak = found == 0 ? summary : (r.text != null ? firstSentence(r.text.note) : summary);
        return reply(summary, speak, "", "", sections, found == 0 || !r.notes.isEmpty() ? r.notes : null);
    }

    private static String firstSentence(String s) {
        int dot = s.indexOf(". ");
        String x = dot > 20 && dot < 300 ? s.substring(0, dot + 1) : s.length() > 240 ? s.substring(0, 240) : s;
        return x;
    }

    private static void addSection(JSONArray sections, String kind, String title, List<WebSearch.Item> items) throws Exception {
        if (items.isEmpty()) return;
        JSONObject s = new JSONObject();
        s.put("kind", kind);
        s.put("title", title);
        JSONArray a = new JSONArray();
        for (WebSearch.Item i : items) {
            JSONObject o = new JSONObject();
            o.put("title", i.title);
            o.put("note", i.note);
            o.put("thumb", i.thumb);
            o.put("url", i.url);
            o.put("host", i.url.isEmpty() ? "" : WebSearch.hostOf(i.url));
            a.put(o);
        }
        s.put("items", a);
        sections.put(s);
    }

    private String findFile(String q, String kindWord) {
        if (q.isEmpty()) return say("Какой файл найти? Например: «найди файл отчёт».");
        String perm = Build.VERSION.SDK_INT >= 33 ? Manifest.permission.READ_MEDIA_IMAGES : Manifest.permission.READ_EXTERNAL_STORAGE;
        String tree = prefs.getString("tree", "");
        boolean media = has(perm);
        boolean docs = kindWord.equals("файл") || kindWord.equals("файлы") || kindWord.equals("документ");
        if (!media && tree.isEmpty()) {
            return need("Чтобы искать файлы, разрешите доступ к фото, видео и музыке или выберите папку для поиска документов. Нажмите кнопку ниже.", "files");
        }
        List<JSONObject> found = new ArrayList<>();
        try {
            if (media) searchMedia(q, kindWord, found);
            if (!tree.isEmpty() && docs) searchTree(Uri.parse(tree), q, found);
        } catch (RuntimeException ignored) {
        }
        if (found.isEmpty()) return say("Файлов с «" + q + "» не нашёл" + (tree.isEmpty() ? " (ищу в фото, видео и музыке; документы — после выбора папки в настройках)." : "."));
        JSONArray sections = new JSONArray();
        try {
            JSONObject s = new JSONObject();
            s.put("kind", "files");
            s.put("title", "📁 Файлы: " + q);
            JSONArray a = new JSONArray();
            for (JSONObject o : found) a.put(o);
            s.put("items", a);
            sections.put(s);
        } catch (Exception ignored) {
        }
        return reply("Нашёл файлов: " + found.size() + ".", "Нашёл файлов: " + found.size() + ".", "", "", sections, null);
    }

    private void searchMedia(String q, String kindWord, List<JSONObject> out) {
        ContentResolver cr = ctx.getContentResolver();
        String sel = MediaStore.Files.FileColumns.DISPLAY_NAME + " LIKE ?";
        List<String> args = new ArrayList<>();
        args.add("%" + q.replace("%", "") + "%");
        if (kindWord.equals("фото")) {
            sel += " AND " + MediaStore.Files.FileColumns.MIME_TYPE + " LIKE ?";
            args.add("image/%");
        } else if (kindWord.equals("видео")) {
            sel += " AND " + MediaStore.Files.FileColumns.MIME_TYPE + " LIKE ?";
            args.add("video/%");
        }
        Cursor c = null;
        try {
            c = cr.query(MediaStore.Files.getContentUri("external"), new String[]{MediaStore.Files.FileColumns._ID, MediaStore.Files.FileColumns.DISPLAY_NAME, MediaStore.Files.FileColumns.MIME_TYPE},
                sel, args.toArray(new String[0]), MediaStore.Files.FileColumns.DATE_MODIFIED + " DESC");
            while (c != null && c.moveToNext() && out.size() < 40) {
                long id = c.getLong(0);
                String mime = c.getString(2) == null ? "" : c.getString(2);
                Uri u = MediaStore.Files.getContentUri("external", id);
                JSONObject o = new JSONObject();
                o.put("title", c.getString(1));
                o.put("note", mime);
                o.put("uri", u.toString());
                o.put("mime", mime);
                if (mime.startsWith("image/") || mime.startsWith("video/")) o.put("thumb", u.toString());
                out.add(o);
            }
        } catch (Exception ignored) {
        } finally {
            if (c != null) c.close();
        }
    }

    // the folder that the owner chose (Storage Access Framework): a quick walk with a limit of entries and depth
    private void searchTree(Uri tree, String q, List<JSONObject> out) {
        String id = DocumentsContract.getTreeDocumentId(tree);
        walk(tree, id, Norm.normalize(q), out, 0, new int[]{0});
    }

    private void walk(Uri tree, String docId, String q, List<JSONObject> out, int depth, int[] seen) {
        if (depth > 4 || seen[0] > 4000 || out.size() >= 40) return;
        Uri children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, docId);
        Cursor c = null;
        List<String[]> dirs = new ArrayList<>();
        try {
            c = ctx.getContentResolver().query(children, new String[]{DocumentsContract.Document.COLUMN_DOCUMENT_ID, DocumentsContract.Document.COLUMN_DISPLAY_NAME, DocumentsContract.Document.COLUMN_MIME_TYPE}, null, null, null);
            while (c != null && c.moveToNext()) {
                seen[0]++;
                String did = c.getString(0), name = c.getString(1), mime = c.getString(2);
                if (DocumentsContract.Document.MIME_TYPE_DIR.equals(mime)) {
                    dirs.add(new String[]{did});
                } else if (name != null && Norm.normalize(name).contains(q) && out.size() < 40) {
                    JSONObject o = new JSONObject();
                    o.put("title", name);
                    o.put("note", mime == null ? "" : mime);
                    o.put("uri", DocumentsContract.buildDocumentUriUsingTree(tree, did).toString());
                    o.put("mime", mime == null ? "" : mime);
                    out.add(o);
                }
            }
        } catch (Exception ignored) {
        } finally {
            if (c != null) c.close();
        }
        for (String[] d : dirs) walk(tree, d[0], q, out, depth + 1, seen);
    }

    /** Opens a file that was found (a content address) with the application that can show it. */
    public String openFile(String uri, String mime) {
        try {
            Intent i = new Intent(Intent.ACTION_VIEW);
            i.setDataAndType(Uri.parse(uri), mime == null || mime.isEmpty() ? "*/*" : mime);
            i.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_ACTIVITY_NEW_TASK);
            ctx.startActivity(i);
            return null;
        } catch (RuntimeException e) {
            return "Нет приложения, которое открывает этот файл.";
        }
    }

    public void saveTree(String uri) {
        prefs.edit().putString("tree", uri).apply();
    }

    /** What is switched on (the page shows buttons for what is not). */
    public String status() {
        try {
            JSONObject o = new JSONObject();
            o.put("mic", has(Manifest.permission.RECORD_AUDIO));
            o.put("contacts", has(Manifest.permission.READ_CONTACTS));
            o.put("call", has(Manifest.permission.CALL_PHONE));
            o.put("media", has(Build.VERSION.SDK_INT >= 33 ? Manifest.permission.READ_MEDIA_IMAGES : Manifest.permission.READ_EXTERNAL_STORAGE));
            o.put("access", JarvisAccess.enabled());
            o.put("tree", !prefs.getString("tree", "").isEmpty());
            o.put("city", prefs.getString("city", ""));
            o.put("ai", !aiKey().isEmpty());
            return o.toString();
        } catch (Exception e) {
            return "{}";
        }
    }

    public void setCity(String city) {
        prefs.edit().putString("city", city).apply();
    }

    // the application is the owner of the activity for the settings screens
    static void openAccessibilitySettings(Activity a) {
        a.startActivity(new Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS));
    }
}
