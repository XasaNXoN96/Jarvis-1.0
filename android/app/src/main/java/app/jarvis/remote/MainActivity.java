package app.jarvis.remote;

import android.Manifest;
import android.app.Activity;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.SharedPreferences;
import android.content.pm.PackageManager;
import android.graphics.Color;
import android.graphics.Typeface;
import android.net.Uri;
import android.net.http.SslError;
import android.os.BatteryManager;
import android.os.Build;
import android.os.Bundle;
import android.speech.RecognitionListener;
import android.speech.RecognizerIntent;
import android.speech.SpeechRecognizer;
import android.text.InputType;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowManager;
import android.webkit.JavascriptInterface;
import android.webkit.PermissionRequest;
import android.webkit.SslErrorHandler;
import android.webkit.WebChromeClient;
import android.webkit.WebResourceRequest;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.Button;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.TextView;

import org.json.JSONObject;

import java.util.ArrayList;

/**
 * Jarvis remote for Android: the same page that the computer serves to a browser (the sphere, the themes, the history),
 * inside the application, plus what a browser cannot do: the speech recognizer of the phone, the battery, the screen kept on.
 * The application talks ONLY to the computer that the user entered. Nothing is sent anywhere else.
 */
public class MainActivity extends Activity {
    private static final String PREFS = "jarvis";
    private static final String KEY_URL = "url";
    private static final int REQ_MIC = 7;

    private SharedPreferences prefs;
    private WebView web;
    private LinearLayout setup;
    private String host = "";
    private SpeechRecognizer recognizer;
    private String pendingLang = "ru-RU";

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        prefs = getSharedPreferences(PREFS, MODE_PRIVATE);
        String saved = prefs.getString(KEY_URL, "");
        if (saved.isEmpty()) showSetup(""); else openRemote(saved);
    }

    // ---------------------------------------------------------------- the first screen: where is the computer
    private int dp(int v) {
        return Math.round(v * getResources().getDisplayMetrics().density);
    }

    private void showSetup(String note) {
        destroyWeb();
        setup = new LinearLayout(this);
        setup.setOrientation(LinearLayout.VERTICAL);
        setup.setGravity(Gravity.CENTER_VERTICAL);
        setup.setBackgroundColor(Color.parseColor("#08090b"));
        setup.setPadding(dp(24), dp(24), dp(24), dp(24));

        TextView title = new TextView(this);
        title.setText("JARVIS");
        title.setTextColor(Color.parseColor("#8ff0e6"));
        title.setTextSize(30);
        title.setTypeface(Typeface.DEFAULT_BOLD);
        title.setLetterSpacing(0.15f);
        setup.addView(title);

        TextView hint = new TextView(this);
        hint.setText("Подключение к компьютеру\n\nНа компьютере откройте Jarvis → Настройки → Подключения → «Пульт телефона». Там показан адрес (например 192.168.1.10) и код привязки. Телефон и компьютер должны быть в одной сети Wi-Fi.\n\nВведите адрес компьютера или вставьте всю ссылку пульта.");
        hint.setTextColor(Color.parseColor("#c7d2d4"));
        hint.setTextSize(15);
        hint.setPadding(0, dp(12), 0, dp(16));
        setup.addView(hint);

        final EditText input = new EditText(this);
        input.setHint("192.168.1.10  или  http://192.168.1.10:9713/?t=…");
        input.setHintTextColor(Color.parseColor("#6b7a7e"));
        input.setTextColor(Color.WHITE);
        input.setSingleLine(true);
        input.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_URI);
        setup.addView(input, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));

        final TextView msg = new TextView(this);
        msg.setTextColor(Color.parseColor("#ff7a7a"));
        msg.setText(note);
        msg.setPadding(0, dp(8), 0, dp(8));
        setup.addView(msg);

        Button go = new Button(this);
        go.setText("Подключиться");
        go.setOnClickListener(v -> {
            String url = Address.normalize(input.getText().toString());
            if (url == null) {
                msg.setText("Не похоже на адрес. Пример: 192.168.1.10");
                return;
            }
            prefs.edit().putString(KEY_URL, url).apply();
            openRemote(url);
        });
        setup.addView(go);
        setContentView(setup);
    }

    // ---------------------------------------------------------------- the page of the remote
    private void openRemote(String url) {
        destroyWeb();
        host = Address.hostOf(url);
        web = new WebView(this);
        web.setBackgroundColor(Color.parseColor("#08090b"));
        WebSettings st = web.getSettings();
        st.setJavaScriptEnabled(true);
        st.setDomStorageEnabled(true);
        st.setMediaPlaybackRequiresUserGesture(false);
        st.setAllowFileAccess(false);
        st.setAllowContentAccess(false);
        st.setUserAgentString(st.getUserAgentString() + " JarvisAndroid/1");
        web.addJavascriptInterface(new Bridge(), "JarvisAndroid");
        web.setWebViewClient(new WebViewClient() {
            @Override
            public boolean shouldOverrideUrlLoading(WebView v, WebResourceRequest r) {
                Uri u = r.getUrl();
                if ("jarvis-app".equals(u.getScheme())) {
                    prefs.edit().remove(KEY_URL).apply();
                    showSetup("");
                    return true;
                }
                if (host != null && host.equalsIgnoreCase(u.getHost())) return false;
                // a link to something else opens in the browser of the phone, never inside the application
                try {
                    startActivity(new Intent(Intent.ACTION_VIEW, u));
                } catch (Exception ignored) {
                }
                return true;
            }

            // the computer makes its own certificate for https: it is accepted ONLY for the computer that was entered
            @Override
            public void onReceivedSslError(WebView v, SslErrorHandler h, SslError e) {
                String eh = Uri.parse(e.getUrl()).getHost();
                if (host != null && host.equalsIgnoreCase(eh)) h.proceed(); else h.cancel();
            }

            @Override
            public void onReceivedError(WebView v, int code, String desc, String failingUrl) {
                if (failingUrl != null && host != null && failingUrl.contains(host)) showOffline(desc);
            }
        });
        web.setWebChromeClient(new WebChromeClient() {
            @Override
            public void onPermissionRequest(PermissionRequest r) {
                // the page may use the microphone only on the computer that was entered
                if (host != null && host.equalsIgnoreCase(r.getOrigin().getHost())) r.grant(r.getResources()); else r.deny();
            }
        });
        setContentView(web, new ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        web.loadUrl(url);
    }

    private void showOffline(String why) {
        runOnUiThread(() -> {
            if (web == null) return;
            String html = "<body style='background:#08090b;color:#ddd;font-family:sans-serif;padding:28px'><h2 style='color:#8ff0e6'>Нет связи с компьютером</h2>"
                    + "<p>Jarvis на компьютере не отвечает. Проверьте: компьютер включён, Jarvis запущен, «Пульт телефона» включён, телефон в той же сети Wi-Fi.</p>"
                    + "<p style='color:#888'>" + android.text.Html.escapeHtml(why == null ? "" : why) + "</p>"
                    + "<p><a style='color:#8ff0e6' href='" + prefs.getString(KEY_URL, "") + "'>Повторить</a></p>"
                    + "<p><a style='color:#8ff0e6' href='jarvis-app://change'>Сменить компьютер</a></p></body>";
            web.loadDataWithBaseURL(prefs.getString(KEY_URL, ""), html, "text/html", "utf-8", null);
        });
    }

    private void destroyWeb() {
        stopListening();
        if (web != null) {
            ViewGroup p = (ViewGroup) web.getParent();
            if (p != null) p.removeView(web);
            web.destroy();
            web = null;
        }
    }

    @Override
    public void onBackPressed() {
        if (web != null && web.canGoBack()) web.goBack(); else moveTaskToBack(true);
    }

    @Override
    protected void onDestroy() {
        stopListening();
        if (recognizer != null) {
            recognizer.destroy();
            recognizer = null;
        }
        super.onDestroy();
    }

    // ---------------------------------------------------------------- what the page can ask of the phone
    private void js(String code) {
        runOnUiThread(() -> {
            if (web != null) web.evaluateJavascript(code, null);
        });
    }

    private void stopListening() {
        if (recognizer != null) {
            try {
                recognizer.cancel();
            } catch (Exception ignored) {
            }
        }
    }

    private void startListening(String lang) {
        if (Build.VERSION.SDK_INT >= 23 && checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            pendingLang = lang;
            requestPermissions(new String[]{Manifest.permission.RECORD_AUDIO}, REQ_MIC);
            return;
        }
        if (!SpeechRecognizer.isRecognitionAvailable(this)) {
            js("window.onNativeSpeechError && onNativeSpeechError('service-not-allowed'); window.onNativeSpeechEnd && onNativeSpeechEnd();");
            return;
        }
        if (recognizer == null) {
            recognizer = SpeechRecognizer.createSpeechRecognizer(this);
            recognizer.setRecognitionListener(new RecognitionListener() {
                @Override public void onReadyForSpeech(Bundle p) { }
                @Override public void onBeginningOfSpeech() { }
                @Override public void onRmsChanged(float v) { }
                @Override public void onBufferReceived(byte[] b) { }
                @Override public void onEndOfSpeech() { }
                @Override public void onEvent(int t, Bundle b) { }

                @Override
                public void onPartialResults(Bundle b) {
                    deliver(b, false);
                }

                @Override
                public void onResults(Bundle b) {
                    deliver(b, true);
                    js("window.onNativeSpeechEnd && onNativeSpeechEnd();");
                }

                @Override
                public void onError(int code) {
                    String e;
                    switch (code) {
                        case SpeechRecognizer.ERROR_NO_MATCH:
                        case SpeechRecognizer.ERROR_SPEECH_TIMEOUT:
                            e = "no-speech";
                            break;
                        case SpeechRecognizer.ERROR_INSUFFICIENT_PERMISSIONS:
                            e = "not-allowed";
                            break;
                        case SpeechRecognizer.ERROR_NETWORK:
                        case SpeechRecognizer.ERROR_NETWORK_TIMEOUT:
                            e = "network";
                            break;
                        case SpeechRecognizer.ERROR_CLIENT:
                            e = "aborted";
                            break;
                        default:
                            e = "other";
                    }
                    js("window.onNativeSpeechError && onNativeSpeechError(" + JSONObject.quote(e) + "); window.onNativeSpeechEnd && onNativeSpeechEnd();");
                }
            });
        }
        Intent i = new Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH);
        i.putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM);
        i.putExtra(RecognizerIntent.EXTRA_LANGUAGE, lang);
        i.putExtra(RecognizerIntent.EXTRA_PARTIAL_RESULTS, true);
        i.putExtra(RecognizerIntent.EXTRA_MAX_RESULTS, 1);
        recognizer.startListening(i);
    }

    private void deliver(Bundle b, boolean last) {
        ArrayList<String> list = b == null ? null : b.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION);
        if (list == null || list.isEmpty()) return;
        js("window.onNativeSpeech && onNativeSpeech(" + JSONObject.quote(list.get(0)) + ", " + last + ");");
    }

    @Override
    public void onRequestPermissionsResult(int req, String[] perms, int[] res) {
        if (req != REQ_MIC) return;
        if (res.length > 0 && res[0] == PackageManager.PERMISSION_GRANTED) {
            startListening(pendingLang);
        } else {
            js("window.onNativeSpeechError && onNativeSpeechError('not-allowed'); window.onNativeSpeechEnd && onNativeSpeechEnd();");
        }
    }

    /** Only the page of the computer that was entered is ever shown in the WebView (other links open in the browser), so only it can call this. */
    private class Bridge {
        @JavascriptInterface
        public void listen(String lang) {
            final String l = lang == null || lang.isEmpty() ? "ru-RU" : lang;
            runOnUiThread(() -> startListening(l));
        }

        @JavascriptInterface
        public void stopListening() {
            runOnUiThread(MainActivity.this::stopListening);
        }

        @JavascriptInterface
        public String battery() {
            Intent b = registerReceiver(null, new IntentFilter(Intent.ACTION_BATTERY_CHANGED));
            if (b == null) return "{}";
            int level = b.getIntExtra(BatteryManager.EXTRA_LEVEL, -1), scale = b.getIntExtra(BatteryManager.EXTRA_SCALE, 100);
            int status = b.getIntExtra(BatteryManager.EXTRA_STATUS, -1);
            boolean charging = status == BatteryManager.BATTERY_STATUS_CHARGING || status == BatteryManager.BATTERY_STATUS_FULL;
            if (level < 0 || scale <= 0) return "{}";
            return "{\"level\":" + (level / (double) scale) + ",\"charging\":" + charging + "}";
        }

        @JavascriptInterface
        public void keepScreenOn(final boolean on) {
            runOnUiThread(() -> {
                if (on) getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
                else getWindow().clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
            });
        }

        @JavascriptInterface
        public void changeComputer() {
            runOnUiThread(() -> {
                prefs.edit().remove(KEY_URL).apply();
                showSetup("");
            });
        }

        @JavascriptInterface
        public String version() {
            return "1.0.0";
        }
    }
}
