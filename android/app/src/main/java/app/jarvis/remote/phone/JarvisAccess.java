package app.jarvis.remote.phone;

import android.accessibilityservice.AccessibilityService;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.view.accessibility.AccessibilityEvent;
import android.view.accessibility.AccessibilityNodeInfo;

/**
 * The accessibility service of Jarvis (OFF until the owner switches it on in the settings of the phone; res/xml/jarvis_access.xml says what it is for).
 * - global actions: lock the screen, a screenshot, home, back (a program cannot do them in any other way);
 * - «Отправить»: after the owner confirmed a message, the chat of Telegram or WhatsApp is open; the service puts the text in the field (when it is not there
 *   yet) and presses the send button. It sees only those two applications.
 */
public class JarvisAccess extends AccessibilityService {
    private static volatile JarvisAccess instance;
    private final Handler handler = new Handler(Looper.getMainLooper());

    @Override
    protected void onServiceConnected() {
        super.onServiceConnected();
        instance = this;
    }

    @Override
    public void onAccessibilityEvent(AccessibilityEvent event) {
        // nothing is read or kept: the service acts only when Jarvis asks
    }

    @Override
    public void onInterrupt() {
    }

    @Override
    public boolean onUnbind(android.content.Intent intent) {
        instance = null;
        return super.onUnbind(intent);
    }

    public static boolean enabled() {
        return instance != null;
    }

    // a global action (GLOBAL_ACTION_LOCK_SCREEN and GLOBAL_ACTION_TAKE_SCREENSHOT need Android 9, API 28)
    public static boolean global(int action) {
        JarvisAccess s = instance;
        return s != null && s.performGlobalAction(action);
    }

    public static boolean canLockAndShoot() {
        return Build.VERSION.SDK_INT >= 28;
    }

    /** Callback of the sending: true = the send button was pressed. */
    public interface Done {
        void done(boolean pressed);
    }

    /**
     * Presses «Отправить» in the open chat. `text` is put into the field first when `setText` is true (Telegram has no text in its link; WhatsApp gets it from the
     * link). It tries for about 15 seconds (the chat takes time to open); when it does not succeed nothing is pressed anywhere else.
     */
    public static void autoSend(final String text, final boolean setText, final Done done) {
        final JarvisAccess s = instance;
        if (s == null) {
            if (done != null) done.done(false);
            return;
        }
        s.handler.postDelayed(new Runnable() {
            int tries = 0;
            boolean typed = !setText;

            @Override
            public void run() {
                tries++;
                boolean pressed = false;
                AccessibilityNodeInfo root = s.getRootInActiveWindow();
                if (root != null) {
                    try {
                        if (!typed) {
                            AccessibilityNodeInfo field = findEditable(root);
                            if (field != null) {
                                Bundle b = new Bundle();
                                b.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, text);
                                typed = field.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, b);
                            }
                        }
                        if (typed) {
                            AccessibilityNodeInfo send = findSend(root);
                            if (send != null) pressed = click(send);
                        }
                    } catch (RuntimeException ignored) {
                        // a window that changed while it was read: the next try
                    }
                }
                if (pressed) {
                    if (done != null) done.done(true);
                } else if (tries >= 20) {
                    if (done != null) done.done(false);
                } else {
                    s.handler.postDelayed(this, 750);
                }
            }
        }, 1500);
    }

    private static AccessibilityNodeInfo findEditable(AccessibilityNodeInfo n) {
        if (n == null) return null;
        if (n.isEditable() && n.getClassName() != null && n.getClassName().toString().contains("EditText")) return n;
        for (int i = 0; i < n.getChildCount(); i++) {
            AccessibilityNodeInfo r = findEditable(n.getChild(i));
            if (r != null) return r;
        }
        return null;
    }

    private static boolean isSend(AccessibilityNodeInfo n) {
        CharSequence d = n.getContentDescription();
        CharSequence t = n.getText();
        String id = n.getViewIdResourceName();
        String s = ((d == null ? "" : d.toString()) + "|" + (t == null ? "" : t.toString())).toLowerCase();
        if (s.contains("send") || s.contains("отправить") || s.contains("отправ")) return !s.contains("voice") && !s.contains("голос");
        return id != null && id.toLowerCase().endsWith(":id/send");
    }

    private static AccessibilityNodeInfo findSend(AccessibilityNodeInfo n) {
        if (n == null) return null;
        if (isSend(n) && !n.isEditable()) return n;
        for (int i = 0; i < n.getChildCount(); i++) {
            AccessibilityNodeInfo r = findSend(n.getChild(i));
            if (r != null) return r;
        }
        return null;
    }

    private static boolean click(AccessibilityNodeInfo n) {
        AccessibilityNodeInfo x = n;
        while (x != null) {
            if (x.isClickable()) return x.performAction(AccessibilityNodeInfo.ACTION_CLICK);
            x = x.getParent();
        }
        return false;
    }
}
