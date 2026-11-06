package app.jarvis.remote.phone;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;

/** The phone was switched on: the reminders that were waiting are set again (the alarms of the phone do not survive a restart). Nothing else is done here. */
public class BootReceiver extends BroadcastReceiver {
    @Override
    public void onReceive(Context ctx, Intent intent) {
        if (intent != null && Intent.ACTION_BOOT_COMPLETED.equals(intent.getAction())) Reminders.rescheduleAll(ctx);
    }
}
