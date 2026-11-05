package app.jarvis.remote.phone;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.content.pm.ServiceInfo;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import android.speech.RecognitionListener;
import android.speech.RecognizerIntent;
import android.speech.SpeechRecognizer;
import android.speech.tts.TextToSpeech;

import org.json.JSONObject;

import java.util.ArrayList;
import java.util.Locale;

/**
 * «Джарвис» in the background (off until the owner switches it on in the settings of Jarvis): a service with a permanent notification (and its button
 * «Выключить») listens with the speech recognizer of the phone; a phrase that begins with the name «Джарвис» is run like a typed command and the answer is
 * spoken. Everything else that was heard is dropped at once. It stops while the screen of Jarvis is open (the microphone is used there).
 * The recognizer of Google may play a short sound at every restart and the battery is used: that is why it is only on by the owner's choice.
 */
public class JarvisListen extends Service {
    static final String CHANNEL = "jarvis_listen", STOP = "app.jarvis.remote.STOP_LISTEN";
    static volatile boolean paused = false;
    private static volatile boolean running = false;
    private final Handler h = new Handler(Looper.getMainLooper());
    private SpeechRecognizer rec;
    private TextToSpeech tts;
    private boolean ttsReady = false, speaking = false;
    private PhoneBrain brain;
    private boolean stopped = false;

    public static boolean running() {
        return running;
    }

    public static void start(Context c) {
        Intent i = new Intent(c, JarvisListen.class);
        if (Build.VERSION.SDK_INT >= 26) c.startForegroundService(i);
        else c.startService(i);
    }

    public static void stop(Context c) {
        c.stopService(new Intent(c, JarvisListen.class));
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent != null && STOP.equals(intent.getAction())) {
            getSharedPreferences("jarvis_phone", Context.MODE_PRIVATE).edit().putBoolean("wake_on", false).apply();
            stopSelf();
            return START_NOT_STICKY;
        }
        if (checkSelfPermission(android.Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            stopSelf();
            return START_NOT_STICKY;
        }
        notifyForeground();
        if (!running) {
            running = true;
            brain = new PhoneBrain(this);
            tts = new TextToSpeech(this, new TextToSpeech.OnInitListener() {
                @Override
                public void onInit(int status) {
                    if (status == TextToSpeech.SUCCESS) {
                        int r = tts.setLanguage(new Locale("ru"));
                        ttsReady = r != TextToSpeech.LANG_MISSING_DATA && r != TextToSpeech.LANG_NOT_SUPPORTED;
                    }
                }
            });
            h.post(loop);
        }
        return START_STICKY;
    }

    private void notifyForeground() {
        NotificationManager nm = (NotificationManager) getSystemService(Context.NOTIFICATION_SERVICE);
        Notification.Builder b;
        if (Build.VERSION.SDK_INT >= 26) {
            nm.createNotificationChannel(new NotificationChannel(CHANNEL, "Джарвис слушает", NotificationManager.IMPORTANCE_LOW));
            b = new Notification.Builder(this, CHANNEL);
        } else {
            b = new Notification.Builder(this);
        }
        PendingIntent stop = PendingIntent.getService(this, 1, new Intent(this, JarvisListen.class).setAction(STOP), PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
        PendingIntent open = PendingIntent.getActivity(this, 2, new Intent(this, PhoneActivity.class), PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
        b.setSmallIcon(getApplicationInfo().icon).setContentTitle("Джарвис слушает").setContentText("Скажите «Джарвис …». Микрофон включён.").setOngoing(true).setContentIntent(open)
            .addAction(new Notification.Action.Builder(0, "Выключить", stop).build());
        Notification n = b.build();
        if (Build.VERSION.SDK_INT >= 29) startForeground(7, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE);
        else startForeground(7, n);
    }

    private final Runnable loop = new Runnable() {
        @Override
        public void run() {
            if (stopped) return;
            if (paused || speaking || !SpeechRecognizer.isRecognitionAvailable(JarvisListen.this)) {
                if (rec != null) { rec.destroy(); rec = null; }
                h.postDelayed(this, 1500);
                return;
            }
            listenOnce();
        }
    };

    private void again(long ms) {
        if (!stopped) h.postDelayed(loop, ms);
    }

    private void listenOnce() {
        if (rec != null) rec.destroy();
        rec = SpeechRecognizer.createSpeechRecognizer(this);
        rec.setRecognitionListener(new RecognitionListener() {
            @Override public void onReadyForSpeech(Bundle p) { }
            @Override public void onBeginningOfSpeech() { }
            @Override public void onRmsChanged(float v) { }
            @Override public void onBufferReceived(byte[] b) { }
            @Override public void onEndOfSpeech() { }
            @Override public void onPartialResults(Bundle b) { }
            @Override public void onEvent(int t, Bundle b) { }

            @Override
            public void onError(int code) {
                // no speech is the normal case: listen again (a longer pause after a real error, so that the battery is not eaten)
                again(code == SpeechRecognizer.ERROR_NO_MATCH || code == SpeechRecognizer.ERROR_SPEECH_TIMEOUT ? 300 : 4000);
            }

            @Override
            public void onResults(Bundle b) {
                ArrayList<String> r = b.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION);
                String heard = r == null || r.isEmpty() ? "" : r.get(0);
                if (Parser.isWake(heard)) handle(heard);
                else again(200);
            }
        });
        Intent i = new Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH);
        i.putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM);
        i.putExtra(RecognizerIntent.EXTRA_LANGUAGE, "ru-RU");
        i.putExtra(RecognizerIntent.EXTRA_MAX_RESULTS, 1);
        try {
            rec.startListening(i);
        } catch (Exception e) {
            again(4000);
        }
    }

    private void handle(final String heard) {
        speaking = true;
        new Thread(new Runnable() {
            @Override
            public void run() {
                String say;
                String rest = heard.trim().replaceFirst("^\\S+\\s*", "");
                if (rest.isEmpty()) say = "Слушаю.";
                else {
                    try {
                        say = new JSONObject(brain.run(heard)).optString("speak", "");
                    } catch (Exception e) {
                        say = "Не получилось.";
                    }
                }
                final String out = say;
                h.post(new Runnable() {
                    @Override
                    public void run() {
                        if (ttsReady && !out.isEmpty()) {
                            tts.speak(out.length() > 400 ? out.substring(0, 400) : out, TextToSpeech.QUEUE_FLUSH, null, "wake");
                            h.postDelayed(new Runnable() {
                                @Override
                                public void run() {
                                    speaking = false;
                                    again(100);
                                }
                            }, Math.min(15000, 1200 + out.length() * 70L));
                        } else {
                            speaking = false;
                            again(100);
                        }
                    }
                });
            }
        }).start();
    }

    @Override
    public void onDestroy() {
        stopped = true;
        running = false;
        h.removeCallbacksAndMessages(null);
        if (rec != null) rec.destroy();
        if (tts != null) tts.shutdown();
        super.onDestroy();
    }
}
