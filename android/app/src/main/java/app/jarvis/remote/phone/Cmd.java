package app.jarvis.remote.phone;

/** What the user asked the phone to do (the result of Parser): a kind and its arguments. */
public final class Cmd {
    public static final String OPEN_APP = "open_app", OPEN_SITE = "open_site", YOUTUBE = "youtube", WEB_SEARCH = "web_search", FIND_FILE = "find_file",
        MUSIC_PLAY = "music_play", MUSIC_PAUSE = "music_pause", MUSIC_NEXT = "music_next", MUSIC_PREV = "music_prev",
        VOL_UP = "vol_up", VOL_DOWN = "vol_down", MUTE = "mute", UNMUTE = "unmute", VOL_SET = "vol_set",
        TIMER = "timer", REMIND = "remind", CHAT = "chat", ALARM = "alarm", TORCH_ON = "torch_on", TORCH_OFF = "torch_off", CALL = "call", SMS = "sms", MESSAGE = "message",
        LOCK = "lock", SCREENSHOT = "screenshot", HOME = "home", BACK = "back", TIME = "time", DATE = "date", BATTERY = "battery", WEATHER = "weather",
        REMEMBER = "remember", RECALL = "recall", FORGET = "forget", SETTINGS = "settings", PC_MODE = "pc_mode", HELP = "help",
        ASK = "ask", UNKNOWN = "unknown";

    public final String kind;
    // the main argument: a name of an app / a site / a query / a contact / a note
    public final String a;
    // the second: the text of a message / a setting name
    public final String b;
    // a messenger: telegram | whatsapp | sms | ""
    public final String channel;
    // a number: seconds of a timer, percent of the volume, hour of an alarm
    public final int n;
    // minutes of an alarm
    public final int m;
    // a question to the user or a note about what was understood (also for ASK / UNKNOWN)
    public final String say;

    public Cmd(String kind, String a, String b, String channel, int n, int m, String say) {
        this.kind = kind;
        this.a = a == null ? "" : a;
        this.b = b == null ? "" : b;
        this.channel = channel == null ? "" : channel;
        this.n = n;
        this.m = m;
        this.say = say == null ? "" : say;
    }

    public static Cmd of(String kind) {
        return new Cmd(kind, "", "", "", 0, 0, "");
    }

    public static Cmd of(String kind, String a) {
        return new Cmd(kind, a, "", "", 0, 0, "");
    }

    public static Cmd ask(String text) {
        return new Cmd(ASK, "", "", "", 0, 0, text);
    }

    @Override
    public String toString() {
        return kind + "(" + a + "|" + b + "|" + channel + "|" + n + ":" + m + ")";
    }
}
