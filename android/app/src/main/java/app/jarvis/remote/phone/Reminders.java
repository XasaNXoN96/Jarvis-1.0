package app.jarvis.remote.phone;

import android.app.AlarmManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.os.Build;

import org.json.JSONArray;
import org.json.JSONObject;

/**
 * The reminders («напомни через …»): an alarm of the phone for each one, and a short list in the private storage of the application, so that
 * the reminders are set again when the phone is switched on after a restart (BootReceiver). A reminder is removed from the list when it has fired.
 */
final class Reminders {
    private Reminders() { }

    private static final String PREFS = "jarvis_phone", KEY = "reminders";
    private static final int KEEP = 50;

    /** Sets the alarm of one reminder (the same call after a restart). */
    static void schedule(Context ctx, int id, long at, String text) {
        Intent i = new Intent(ctx, ReminderReceiver.class).putExtra("text", text).putExtra("id", id);
        PendingIntent pi = PendingIntent.getBroadcast(ctx, id, i, PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
        AlarmManager am = (AlarmManager) ctx.getSystemService(Context.ALARM_SERVICE);
        if (Build.VERSION.SDK_INT >= 31 && !am.canScheduleExactAlarms()) am.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, pi);
        else am.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, pi);
    }

    private static JSONArray load(SharedPreferences p) {
        try {
            return new JSONArray(p.getString(KEY, "[]"));
        } catch (Exception e) {
            return new JSONArray();
        }
    }

    /** Remembers a reminder (the oldest are dropped past 50). */
    static void add(Context ctx, int id, long at, String text) {
        try {
            SharedPreferences p = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
            JSONArray old = load(p), now = new JSONArray();
            int from = Math.max(0, old.length() - (KEEP - 1));
            for (int k = from; k < old.length(); k++) {
                if (old.getJSONObject(k).optInt("id") != id) now.put(old.getJSONObject(k));
            }
            now.put(new JSONObject().put("id", id).put("at", at).put("text", text));
            p.edit().putString(KEY, now.toString()).apply();
        } catch (Exception ignored) {
            // a list that cannot be written only means that the reminder will not survive a restart
        }
    }

    /** Forgets a reminder (it has fired). */
    static void remove(Context ctx, int id) {
        try {
            SharedPreferences p = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
            JSONArray old = load(p), now = new JSONArray();
            for (int k = 0; k < old.length(); k++) {
                if (old.getJSONObject(k).optInt("id") != id) now.put(old.getJSONObject(k));
            }
            p.edit().putString(KEY, now.toString()).apply();
        } catch (Exception ignored) {
            // nothing to do
        }
    }

    /** After a restart: every reminder of the list is set again; one whose time has passed fires in a few seconds, with a note that it was missed. */
    static void rescheduleAll(Context ctx) {
        try {
            SharedPreferences p = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
            JSONArray list = load(p);
            long now = System.currentTimeMillis();
            for (int k = 0; k < list.length(); k++) {
                JSONObject o = list.getJSONObject(k);
                long at = o.optLong("at");
                String text = o.optString("text", "");
                int id = o.optInt("id");
                if (at <= now) schedule(ctx, id, now + 5000L + k * 1500L, text + " (пропущено: телефон был выключен)");
                else schedule(ctx, id, at, text);
            }
        } catch (Exception ignored) {
            // a reminder that cannot be set again is lost; the others are not affected
        }
    }
}
