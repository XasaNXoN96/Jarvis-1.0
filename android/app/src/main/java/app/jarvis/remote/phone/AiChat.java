package app.jarvis.remote.phone;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.util.List;

import org.json.JSONArray;
import org.json.JSONObject;

/**
 * The AI answer on the phone: the question goes to the OpenAI chat API with the key that the owner typed on this phone (it is kept only in the private
 * storage of the application and is never shown again, logged or sent anywhere else). The body and the answer are pure Java (tested without a phone).
 */
public final class AiChat {
    private AiChat() { }

    public static final String MODEL = "gpt-4o-mini";
    static final String SYSTEM = "Ты Джарвис, голосовой помощник на телефоне. Отвечай по-русски, коротко и по делу (1–4 предложения), без разметки, списков и эмодзи: ответ будут читать вслух.";

    /** history: pairs {question, answer} of the last turns, oldest first. */
    static String body(String question, List<String[]> history) throws Exception {
        JSONArray msgs = new JSONArray();
        msgs.put(new JSONObject().put("role", "system").put("content", SYSTEM));
        for (String[] h : history) {
            msgs.put(new JSONObject().put("role", "user").put("content", h[0]));
            msgs.put(new JSONObject().put("role", "assistant").put("content", h[1]));
        }
        msgs.put(new JSONObject().put("role", "user").put("content", question));
        return new JSONObject().put("model", MODEL).put("messages", msgs).put("max_tokens", 400).put("temperature", 0.6).toString();
    }

    /** The text of the answer, or null when the reply has none. */
    static String answer(String json) {
        try {
            JSONArray ch = new JSONObject(json).optJSONArray("choices");
            if (ch == null || ch.length() == 0) return null;
            JSONObject m = ch.getJSONObject(0).optJSONObject("message");
            String t = m == null ? null : m.optString("content", null);
            return t == null || t.trim().isEmpty() ? null : t.trim();
        } catch (Exception e) {
            return null;
        }
    }

    /** A reason in the words of the owner for an error of the service (the text of the key is never in it). */
    static String reason(int code, String json) {
        if (code == 401) return "ключ не подошёл: проверьте его в настройках";
        if (code == 429) return "лимит или нет денег на счёте OpenAI";
        if (code == 403) return "доступ закрыт для этого ключа или региона";
        if (code >= 500) return "сервис OpenAI сейчас не отвечает";
        String msg = "";
        try {
            JSONObject e = new JSONObject(json).optJSONObject("error");
            if (e != null) msg = e.optString("message", "");
        } catch (Exception ignored) {
        }
        if (msg.length() > 120) msg = msg.substring(0, 120);
        return "ошибка " + code + (msg.isEmpty() ? "" : ": " + msg.replaceAll("sk-[A-Za-z0-9_\\-]+", "…"));
    }

    /** Asks the service. Returns the answer; throws IllegalStateException with a reason for the owner. Call it off the main thread. */
    public static String ask(String key, String question, List<String[]> history) {
        HttpURLConnection c = null;
        try {
            c = (HttpURLConnection) new URL("https://api.openai.com/v1/chat/completions").openConnection();
            c.setConnectTimeout(10_000);
            c.setReadTimeout(40_000);
            c.setRequestMethod("POST");
            c.setDoOutput(true);
            c.setRequestProperty("Content-Type", "application/json");
            c.setRequestProperty("Authorization", "Bearer " + key);
            byte[] b = body(question, history).getBytes(StandardCharsets.UTF_8);
            OutputStream os = c.getOutputStream();
            os.write(b);
            os.close();
            int code = c.getResponseCode();
            InputStream in = code >= 400 ? c.getErrorStream() : c.getInputStream();
            ByteArrayOutputStream bo = new ByteArrayOutputStream();
            if (in != null) {
                byte[] buf = new byte[4096];
                int n;
                while ((n = in.read(buf)) > 0 && bo.size() < 200_000) bo.write(buf, 0, n);
                in.close();
            }
            String json = new String(bo.toByteArray(), StandardCharsets.UTF_8);
            if (code >= 400) throw new IllegalStateException(reason(code, json));
            String a = answer(json);
            if (a == null) throw new IllegalStateException("пустой ответ");
            return a;
        } catch (IllegalStateException e) {
            throw e;
        } catch (java.net.SocketTimeoutException e) {
            throw new IllegalStateException("нет ответа вовремя");
        } catch (Exception e) {
            throw new IllegalStateException("нет связи с интернетом");
        } finally {
            if (c != null) c.disconnect();
        }
    }
}
