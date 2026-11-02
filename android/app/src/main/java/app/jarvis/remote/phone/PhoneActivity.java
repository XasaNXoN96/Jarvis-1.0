package app.jarvis.remote.phone;

import android.Manifest;
import android.app.Activity;
import android.content.ActivityNotFoundException;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.graphics.Color;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.speech.RecognitionListener;
import android.speech.RecognizerIntent;
import android.speech.SpeechRecognizer;
import android.speech.tts.TextToSpeech;
import android.view.ViewGroup;
import android.webkit.JavascriptInterface;
import android.webkit.WebResourceRequest;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;

import app.jarvis.remote.MainActivity;

import org.json.JSONObject;

import java.util.ArrayList;
import java.util.Locale;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * The phone mode of Jarvis: works WITHOUT the computer. The screen is a page of the application itself (assets/phone.html: the sphere, the field, the
 * answers); the commands are carried out by PhoneBrain on this phone; the voice is the speech recognizer and the voice of the phone. The remote of the
 * computer (MainActivity) is one button away. The page can only ask the application to do things through the small bridge below; it can not leave the
 * application (every other address opens in the browser).
 */
public class PhoneActivity extends Activity {
    private static final int REQ_PERMS = 21, REQ_TREE = 22;
    private WebView web;
    private PhoneBrain brain;
    private TextToSpeech tts;
    private boolean ttsReady = false;
    private SpeechRecognizer recognizer;
    private final ExecutorService worker = Executors.newSingleThreadExecutor();

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
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
        web = new WebView(this);
        web.setBackgroundColor(Color.parseColor("#08090b"));
        WebSettings s = web.getSettings();
        s.setJavaScriptEnabled(true);
        s.setDomStorageEnabled(true);
        s.setAllowFileAccess(false);
        s.setAllowContentAccess(false);
        s.setMediaPlaybackRequiresUserGesture(true);
        web.addJavascriptInterface(new Bridge(), "JarvisPhone");
        web.setWebViewClient(new WebViewClient() {
            @Override
            public boolean shouldOverrideUrlLoading(WebView v, WebResourceRequest r) {
                // the page never leaves the application: any other address is opened in the browser
                String u = r.getUrl().toString();
                if (u.startsWith("file:///android_asset/")) return false;
                String safe = WebSearch.safeUrl(u);
                if (safe != null) openExternal(safe);
                return true;
            }
        });
        setContentView(web, new ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        web.loadUrl("file:///android_asset/phone.html");
    }

    private void openExternal(String url) {
        try {
            startActivity(new Intent(Intent.ACTION_VIEW, Uri.parse(url)));
        } catch (ActivityNotFoundException ignored) {
        }
    }

    private void js(final String code) {
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                if (web != null) web.evaluateJavascript(code, null);
            }
        });
    }

    // ---------------------------------------------------------------- the voice
    private void startListening() {
        if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(new String[]{Manifest.permission.RECORD_AUDIO}, REQ_PERMS);
            return;
        }
        if (!SpeechRecognizer.isRecognitionAvailable(this)) {
            js("jarvisSpeechError('На телефоне нет службы распознавания речи (Google). Печатайте команду в поле.')");
            return;
        }
        if (recognizer != null) recognizer.destroy();
        recognizer = SpeechRecognizer.createSpeechRecognizer(this);
        recognizer.setRecognitionListener(new RecognitionListener() {
            @Override public void onReadyForSpeech(Bundle p) { js("jarvisListening(true)"); }
            @Override public void onBeginningOfSpeech() { }
            @Override public void onRmsChanged(float v) { }
            @Override public void onBufferReceived(byte[] b) { }
            @Override public void onEndOfSpeech() { js("jarvisListening(false)"); }
            @Override public void onEvent(int t, Bundle b) { }

            @Override
            public void onPartialResults(Bundle b) {
                deliver(b, false);
            }

            @Override
            public void onResults(Bundle b) {
                deliver(b, true);
            }

            @Override
            public void onError(int code) {
                js("jarvisListening(false)");
                if (code == SpeechRecognizer.ERROR_NO_MATCH || code == SpeechRecognizer.ERROR_SPEECH_TIMEOUT) js("jarvisSpeechError('Не расслышал. Нажмите на сферу и скажите ещё раз.')");
                else if (code == SpeechRecognizer.ERROR_INSUFFICIENT_PERMISSIONS) js("jarvisSpeechError('Нет разрешения на микрофон.')");
                else if (code == SpeechRecognizer.ERROR_NETWORK || code == SpeechRecognizer.ERROR_NETWORK_TIMEOUT) js("jarvisSpeechError('Распознаванию речи нужен интернет (Google). Печатайте команду.')");
                else js("jarvisSpeechError('Ошибка распознавания речи (" + code + ")')");
            }
        });
        Intent i = new Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH);
        i.putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM);
        i.putExtra(RecognizerIntent.EXTRA_LANGUAGE, "ru-RU");
        i.putExtra(RecognizerIntent.EXTRA_PARTIAL_RESULTS, true);
        i.putExtra(RecognizerIntent.EXTRA_MAX_RESULTS, 1);
        recognizer.startListening(i);
    }

    private void deliver(Bundle b, boolean last) {
        ArrayList<String> r = b.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION);
        if (r == null || r.isEmpty()) return;
        js("jarvisHeard(" + JSONObject.quote(r.get(0)) + "," + last + ")");
    }

    @Override
    public void onRequestPermissionsResult(int req, String[] perms, int[] res) {
        super.onRequestPermissionsResult(req, perms, res);
        js("jarvisPermsChanged()");
    }

    @Override
    protected void onActivityResult(int req, int code, Intent data) {
        super.onActivityResult(req, code, data);
        if (req == REQ_TREE && code == RESULT_OK && data != null && data.getData() != null) {
            Uri u = data.getData();
            try {
                getContentResolver().takePersistableUriPermission(u, Intent.FLAG_GRANT_READ_URI_PERMISSION);
            } catch (RuntimeException ignored) {
            }
            brain.saveTree(u.toString());
            js("jarvisPermsChanged()");
        }
    }

    @Override
    protected void onResume() {
        super.onResume();
        js("jarvisPermsChanged()");
    }

    @Override
    protected void onDestroy() {
        if (recognizer != null) recognizer.destroy();
        if (tts != null) {
            tts.stop();
            tts.shutdown();
        }
        worker.shutdownNow();
        if (web != null) {
            ((ViewGroup) web.getParent()).removeView(web);
            web.destroy();
            web = null;
        }
        super.onDestroy();
    }

    // ---------------------------------------------------------------- what the page may ask
    private class Bridge {
        @JavascriptInterface
        public void run(final String text) {
            worker.execute(new Runnable() {
                @Override
                public void run() {
                    String answer = brain.run(text);
                    js("jarvisReply(" + answer + ")");
                }
            });
        }

        @JavascriptInterface
        public void listen() {
            runOnUiThread(new Runnable() {
                @Override
                public void run() {
                    startListening();
                }
            });
        }

        @JavascriptInterface
        public void stopListening() {
            runOnUiThread(new Runnable() {
                @Override
                public void run() {
                    if (recognizer != null) recognizer.stopListening();
                }
            });
        }

        @JavascriptInterface
        public void speak(final String text) {
            runOnUiThread(new Runnable() {
                @Override
                public void run() {
                    if (ttsReady && text != null && !text.isEmpty()) tts.speak(text, TextToSpeech.QUEUE_FLUSH, null, "jarvis");
                }
            });
        }

        @JavascriptInterface
        public void stopSpeaking() {
            runOnUiThread(new Runnable() {
                @Override
                public void run() {
                    if (tts != null) tts.stop();
                }
            });
        }

        @JavascriptInterface
        public String status() {
            return brain.status();
        }

        @JavascriptInterface
        public void request(final String what) {
            runOnUiThread(new Runnable() {
                @Override
                public void run() {
                    switch (what) {
                        case "mic": requestPermissions(new String[]{Manifest.permission.RECORD_AUDIO}, REQ_PERMS); break;
                        case "contacts": requestPermissions(new String[]{Manifest.permission.READ_CONTACTS}, REQ_PERMS); break;
                        case "call": requestPermissions(new String[]{Manifest.permission.CALL_PHONE}, REQ_PERMS); break;
                        case "media":
                        case "files":
                            if (Build.VERSION.SDK_INT >= 33) {
                                requestPermissions(new String[]{Manifest.permission.READ_MEDIA_IMAGES, Manifest.permission.READ_MEDIA_VIDEO, Manifest.permission.READ_MEDIA_AUDIO}, REQ_PERMS);
                            } else {
                                requestPermissions(new String[]{Manifest.permission.READ_EXTERNAL_STORAGE}, REQ_PERMS);
                            }
                            break;
                        case "folder":
                            try {
                                startActivityForResult(new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE), REQ_TREE);
                            } catch (ActivityNotFoundException ignored) {
                            }
                            break;
                        case "access":
                            startActivity(new Intent(android.provider.Settings.ACTION_ACCESSIBILITY_SETTINGS));
                            break;
                        default: break;
                    }
                }
            });
        }

        @JavascriptInterface
        public void openUrl(final String url) {
            final String safe = WebSearch.safeUrl(url);
            if (safe == null) return;
            runOnUiThread(new Runnable() {
                @Override
                public void run() {
                    openExternal(safe);
                }
            });
        }

        @JavascriptInterface
        public void openFile(final String uri, final String mime) {
            runOnUiThread(new Runnable() {
                @Override
                public void run() {
                    String err = brain.openFile(uri, mime);
                    if (err != null) js("jarvisSpeechError(" + JSONObject.quote(err) + ")");
                }
            });
        }

        @JavascriptInterface
        public void openPc() {
            runOnUiThread(new Runnable() {
                @Override
                public void run() {
                    startActivity(new Intent(PhoneActivity.this, MainActivity.class));
                }
            });
        }

        @JavascriptInterface
        public void setCity(String city) {
            brain.setCity(city == null ? "" : city.trim());
        }

        @JavascriptInterface
        public String version() {
            return "phone-1";
        }
    }
}
