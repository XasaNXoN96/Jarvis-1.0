package app.jarvis.remote;

import android.Manifest;
import android.app.Activity;
import android.app.AlertDialog;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.SharedPreferences;
import android.content.pm.PackageManager;
import android.graphics.Color;
import android.graphics.Typeface;
import android.net.Uri;
import android.net.http.SslCertificate;
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

import java.io.ByteArrayInputStream;
import java.security.cert.CertificateFactory;
import java.security.cert.X509Certificate;
import java.util.ArrayList;

/**
 * Jarvis remote for Android: the same page that the computer serves to a browser (the sphere, the themes, the history),
 * inside the application, plus what a browser cannot do: the speech recognizer of the phone, the battery, the screen kept on.
 * The application talks ONLY to the computer that the user entered. Nothing is sent anywhere else.
 */
public class MainActivity extends Activity {
    private static final String PREFS = "jarvis";
    private static final String KEY_URL = "url";
    // the pin of the computer ("sha256/…" of its public key) and the secret of this device: kept by the application, not by the page
    private static final String KEY_PIN = "pin";
    private static final String KEY_TOKEN = "device_token";
    private static final int REQ_MIC = 7;

    private SharedPreferences prefs;
    private WebView web;
    private LinearLayout setup;
    private String host = "";
    private SpeechRecognizer recognizer;
    private String pendingLang = "ru-RU";
    // from a pairing link that was just opened (never stored before the first handshake proved that the computer holds that key)
    private String expectedPin = null;
    private String pairCode = null;
    // handshakes that wait for the owner to compare the fingerprint (nothing is sent to the computer meanwhile)
    private final ArrayList<SslErrorHandler> waiting = new ArrayList<>();
    private boolean asking = false;

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        prefs = getSharedPreferences(PREFS, MODE_PRIVATE);
        String saved = prefs.getString(KEY_URL, "");
        if (saved.isEmpty()) {
            showSetup("");
        } else if (Address.normalize(saved) == null) {
            // an address saved by an older version (plain http): it is never opened; the owner enters the address again (https)
            prefs.edit().remove(KEY_URL).apply();
            showSetup("Сохранённый адрес использовал небезопасный http. Введите адрес компьютера заново: теперь соединение шифруется.");
        } else {
            openRemote(saved);
        }
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
        hint.setText("Подключение к компьютеру\n\nНа компьютере откройте Jarvis → Настройки → Подключения → «Пульт телефона», нажмите «Привязать телефон по коду». Там показаны адрес (например 192.168.1.10), код и отпечаток компьютера. Телефон и компьютер должны быть в одной сети.\n\nВведите адрес компьютера или вставьте ссылку из QR-кода (в ней есть отпечаток). Соединение всегда зашифровано; обычный http не используется.");
        hint.setTextColor(Color.parseColor("#c7d2d4"));
        hint.setTextSize(15);
        hint.setPadding(0, dp(12), 0, dp(16));
        setup.addView(hint);

        final EditText input = new EditText(this);
        input.setHint("192.168.1.10  или  https://192.168.1.10:9713/#pin=…");
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
            Address.Parsed p = Address.parse(input.getText().toString());
            if (p == null) {
                msg.setText("Не похоже на адрес или ссылка повреждена (обычный http не принимается). Пример: 192.168.1.10");
                return;
            }
            if (p.pin != null) {
                // a pairing link from the window of the computer: it is the owner's own decision to trust THIS key; the old pin is replaced,
                // and the new one is stored only after the first handshake showed the same key
                prefs.edit().remove(KEY_PIN).remove(KEY_TOKEN).apply();
                expectedPin = p.pin;
                pairCode = p.code;
            }
            prefs.edit().putString(KEY_URL, p.url).apply();
            openRemote(p.url);
        });
        setup.addView(go);

        // the computer was replaced or its key was changed on purpose: forget the old fingerprint and the secret of this device
        Button forget = new Button(this);
        forget.setText("Забыть прежний компьютер (сбросить отпечаток)");
        forget.setOnClickListener(v -> {
            forgetComputer();
            msg.setText("Отпечаток и секрет этого телефона удалены. Введите адрес: отпечаток нового компьютера нужно будет сверить.");
        });
        setup.addView(forget);
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
                    if ("forget".equals(u.getHost())) {
                        forgetComputer();
                        prefs.edit().remove(KEY_URL).apply();
                        showSetup("Прежний отпечаток удалён. Введите адрес: отпечаток нужно будет сверить заново.");
                        return true;
                    }
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

            // The computer makes its own certificate. There is NO «proceed anyway»: the connection goes on only if the public key of the
            // certificate is the pinned one (or the one in the pairing link, or the one the owner confirmed by comparing the fingerprint)
            // and the certificate is inside its term. Every other case is cancelled.
            @Override
            public void onReceivedSslError(WebView v, SslErrorHandler h, SslError e) {
                String eh = Uri.parse(e.getUrl()).getHost();
                if (host == null || !host.equalsIgnoreCase(eh)) {
                    h.cancel();
                    return;
                }
                X509Certificate leaf = x509(e.getCertificate());
                PinPolicy.Verdict verdict = PinPolicy.evaluate(leaf, prefs.getString(KEY_PIN, null), expectedPin, System.currentTimeMillis(), e.hasError(SslError.SSL_INVALID));
                switch (verdict) {
                    case ACCEPT:
                        h.proceed();
                        return;
                    case ACCEPT_AND_REMEMBER:
                        prefs.edit().putString(KEY_PIN, Pin.of(leaf)).apply();
                        expectedPin = null;
                        h.proceed();
                        return;
                    case ASK_USER:
                        askFingerprint(h, leaf);
                        return;
                    default:
                        h.cancel();
                        showRefused(verdict);
                }
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
        // the pairing code from a link goes into the page as a fragment («#c=…», never sent over the network); only once
        String first = pairCode != null ? url + "#c=" + pairCode : url;
        pairCode = null;
        web.loadUrl(first);
    }

    private void forgetComputer() {
        prefs.edit().remove(KEY_PIN).remove(KEY_TOKEN).apply();
        expectedPin = null;
        pairCode = null;
    }

    // the system gives the certificate as SslCertificate: its X.509 form is needed to read the key
    private static X509Certificate x509(SslCertificate c) {
        if (c == null) return null;
        try {
            if (Build.VERSION.SDK_INT >= 29) return c.getX509Certificate();
            Bundle b = SslCertificate.saveState(c);
            byte[] der = b == null ? null : b.getByteArray("x509-certificate");
            if (der == null) return null;
            return (X509Certificate) CertificateFactory.getInstance("X.509").generateCertificate(new ByteArrayInputStream(der));
        } catch (Exception e) {
            return null;
        }
    }

    // First contact with nothing pinned and no pairing link: the owner compares the fingerprint with the window of the computer. Until he
    // confirms, the handshake is held: the page is not loaded, so the page cannot send the pairing code to a computer that is not checked.
    private void askFingerprint(final SslErrorHandler h, final X509Certificate leaf) {
        waiting.add(h);
        if (asking) return;
        asking = true;
        final String shown = Pin.groups(leaf);
        final String pin = Pin.of(leaf);
        runOnUiThread(() -> new AlertDialog.Builder(this)
                .setTitle("Проверьте отпечаток компьютера")
                .setMessage("Отпечаток:\n\n" + shown + "\n\nСверьте его с окном Jarvis на компьютере (Настройки → Пульт телефона → Безопасность соединения). Если совпадает — подтвердите. Пока вы не подтвердите, ничего не отправляется на компьютер.")
                .setCancelable(false)
                .setPositiveButton("Совпадает", (d, w) -> {
                    prefs.edit().putString(KEY_PIN, pin).apply();
                    asking = false;
                    for (SslErrorHandler x : waiting) x.proceed();
                    waiting.clear();
                })
                .setNegativeButton("Не совпадает", (d, w) -> {
                    asking = false;
                    for (SslErrorHandler x : waiting) x.cancel();
                    waiting.clear();
                    showSetup("Отпечаток не совпал: соединение закрыто. Это может быть другой компьютер или подмена в сети.");
                })
                .show());
    }

    private void showRefused(PinPolicy.Verdict why) {
        runOnUiThread(() -> {
            if (web == null) return;
            String html = "<body style='background:#08090b;color:#ddd;font-family:sans-serif;padding:28px'><h2 style='color:#ff7a7a'>Соединение не открыто</h2>"
                    + "<p>" + android.text.Html.escapeHtml(PinPolicy.refusal(why)) + "</p>"
                    + "<p><a style='color:#8ff0e6' href='" + prefs.getString(KEY_URL, "") + "'>Повторить</a></p>"
                    + (why == PinPolicy.Verdict.MISMATCH ? "<p><a style='color:#8ff0e6' href='jarvis-app://forget'>Это другой компьютер: забыть прежний отпечаток</a></p>" : "")
                    + "<p><a style='color:#8ff0e6' href='jarvis-app://change'>Сменить компьютер</a></p></body>";
            web.loadDataWithBaseURL("about:blank", html, "text/html", "utf-8", null);
        });
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

        // the secret of this device lives here, not in the localStorage of the page (that one belongs to one address of the computer)
        @JavascriptInterface
        public String getToken() {
            return prefs.getString(KEY_TOKEN, "");
        }

        @JavascriptInterface
        public void setToken(String t) {
            if (t == null || t.length() > 200) return;
            if (t.isEmpty()) prefs.edit().remove(KEY_TOKEN).apply(); else prefs.edit().putString(KEY_TOKEN, t).apply();
        }

        @JavascriptInterface
        public String version() {
            return "1.0.0";
        }
    }
}
