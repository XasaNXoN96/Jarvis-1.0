package app.jarvis.remote.phone;

import android.app.Notification;
import android.content.Context;
import android.content.SharedPreferences;
import android.os.Bundle;
import android.service.notification.NotificationListenerService;
import android.service.notification.StatusBarNotification;
import android.speech.tts.TextToSpeech;

import java.util.Locale;

/**
 * Reads new messages aloud: ONLY Telegram, WhatsApp and the SMS application, ONLY when the owner switched «Читать сообщения вслух» on in the settings of
 * Jarvis AND gave the access to notifications in the settings of the phone. Nothing is stored or sent anywhere; the text is spoken by the voice of the phone.
 */
public class JarvisNotify extends NotificationListenerService {
    static final String[] PACKAGES = {"org.telegram.messenger", "org.telegram.messenger.web", "com.whatsapp", "com.whatsapp.w4b", "com.google.android.apps.messaging", "com.samsung.android.messaging"};
    private TextToSpeech tts;
    private boolean ready = false;
    private String lastKey = "";

    @Override
    public void onCreate() {
        super.onCreate();
        tts = new TextToSpeech(this, new TextToSpeech.OnInitListener() {
            @Override
            public void onInit(int status) {
                if (status == TextToSpeech.SUCCESS) {
                    int r = tts.setLanguage(new Locale("ru"));
                    ready = r != TextToSpeech.LANG_MISSING_DATA && r != TextToSpeech.LANG_NOT_SUPPORTED;
                }
            }
        });
    }

    @Override
    public void onDestroy() {
        if (tts != null) tts.shutdown();
        super.onDestroy();
    }

    @Override
    public void onNotificationPosted(StatusBarNotification sbn) {
        try {
            SharedPreferences p = getSharedPreferences("jarvis_phone", Context.MODE_PRIVATE);
            if (!p.getBoolean("read_notif", false) || !ready || sbn == null) return;
            boolean ok = false;
            for (String k : PACKAGES) if (k.equals(sbn.getPackageName())) ok = true;
            if (!ok) return;
            Notification n = sbn.getNotification();
            if (n == null || (n.flags & Notification.FLAG_GROUP_SUMMARY) != 0 || (n.flags & Notification.FLAG_ONGOING_EVENT) != 0) return;
            Bundle e = n.extras;
            if (e == null) return;
            CharSequence title = e.getCharSequence(Notification.EXTRA_TITLE), text = e.getCharSequence(Notification.EXTRA_TEXT);
            if (text == null || text.toString().trim().isEmpty()) return;
            String say = (title == null ? "" : title.toString().trim() + ": ") + text.toString().trim();
            if (say.length() > 300) say = say.substring(0, 300);
            if (say.equals(lastKey)) return;
            lastKey = say;
            tts.speak("Сообщение. " + say, TextToSpeech.QUEUE_ADD, null, "notif" + sbn.getKey());
        } catch (Exception ignored) {
            // a notification that cannot be read is skipped
        }
    }

    /** True when the owner gave this application the access to notifications in the settings of the phone. */
    public static boolean granted(Context c) {
        String s = android.provider.Settings.Secure.getString(c.getContentResolver(), "enabled_notification_listeners");
        return s != null && s.contains(c.getPackageName());
    }
}
