package app.jarvis.remote.phone;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.os.Build;

/** Shows the reminder that «напомни …» set (the time came). Does nothing else; no network, no data leaves the phone. */
public class ReminderReceiver extends BroadcastReceiver {
    static final String CHANNEL = "jarvis_reminders";

    @Override
    public void onReceive(Context ctx, Intent intent) {
        String text = intent.getStringExtra("text");
        if (text == null || text.isEmpty()) text = "Напоминание";
        int id = intent.getIntExtra("id", 1);
        NotificationManager nm = (NotificationManager) ctx.getSystemService(Context.NOTIFICATION_SERVICE);
        if (nm == null) return;
        Notification.Builder b;
        if (Build.VERSION.SDK_INT >= 26) {
            nm.createNotificationChannel(new NotificationChannel(CHANNEL, "Напоминания Jarvis", NotificationManager.IMPORTANCE_HIGH));
            b = new Notification.Builder(ctx, CHANNEL);
        } else {
            b = new Notification.Builder(ctx).setPriority(Notification.PRIORITY_HIGH).setDefaults(Notification.DEFAULT_ALL);
        }
        Intent open = new Intent(ctx, PhoneActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        b.setSmallIcon(ctx.getApplicationInfo().icon).setContentTitle("Jarvis напоминает").setContentText(text).setStyle(new Notification.BigTextStyle().bigText(text))
            .setAutoCancel(true).setContentIntent(android.app.PendingIntent.getActivity(ctx, id, open, android.app.PendingIntent.FLAG_UPDATE_CURRENT | android.app.PendingIntent.FLAG_IMMUTABLE));
        try {
            nm.notify(id, b.build());
        } catch (Exception ignored) {
            // no permission to show notifications: nothing to do
        }
    }
}
