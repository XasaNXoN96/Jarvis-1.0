package app.jarvis.remote.phone;

import java.io.BufferedReader;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.net.HttpURLConnection;
import java.net.URL;
import java.net.URLDecoder;
import java.net.URLEncoder;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.Callable;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.TimeUnit;

import org.json.JSONArray;
import org.json.JSONObject;

/**
 * «найди …» on the phone: the same four kinds of what was found as on the computer, each of them its own list: photos (Wikimedia Commons),
 * videos (the search page of YouTube), links (DuckDuckGo, then Wikipedia), text (the article of Wikipedia). No key and no account;
 * only the text of the request leaves the phone. The parsers are pure Java (tested without a phone).
 */
public final class WebSearch {
    private WebSearch() { }

    /** One thing found: a title, an address, a picture to draw (https), the words under the title. */
    public static final class Item {
        public final String title, url, thumb, note;

        public Item(String title, String url, String thumb, String note) {
            this.title = title;
            this.url = url;
            this.thumb = thumb == null ? "" : thumb;
            this.note = note == null ? "" : note;
        }
    }

    public static final class Result {
        public final List<Item> photos = new ArrayList<>(), videos = new ArrayList<>(), links = new ArrayList<>();
        public Item text;
        public final List<String> notes = new ArrayList<>();   // why a kind is empty
    }

    public static final int MAX_LINKS = 10, MAX_PHOTOS = 24, MAX_VIDEOS = 8;

    static String enc(String s) {
        try {
            return URLEncoder.encode(s, "UTF-8").replace("+", "%20");
        } catch (Exception e) {
            return "";
        }
    }

    static String dec(String s) {
        try {
            return URLDecoder.decode(s, "UTF-8");
        } catch (Exception e) {
            return s;
        }
    }

    // an address that may be shown and opened: http(s), a host with a dot, no login and password
    public static String safeUrl(String u) {
        if (u == null) return null;
        String rest;
        if (u.startsWith("https://")) rest = u.substring(8);
        else if (u.startsWith("http://")) rest = u.substring(7);
        else return null;
        int end = rest.length();
        for (int i = 0; i < rest.length(); i++) {
            char c = rest.charAt(i);
            if (c == '/' || c == '?' || c == '#') {
                end = i;
                break;
            }
        }
        String host = rest.substring(0, end);
        if (host.isEmpty() || host.indexOf('@') >= 0 || host.indexOf('.') < 0 || u.length() > 2000) return null;
        for (int i = 0; i < u.length(); i++) if (Character.isWhitespace(u.charAt(i)) || Character.isISOControl(u.charAt(i))) return null;
        return u;
    }

    public static String hostOf(String url) {
        String rest = url.replaceFirst("^https?://", "");
        int end = rest.length();
        for (int i = 0; i < rest.length(); i++) {
            char c = rest.charAt(i);
            if (c == '/' || c == '?' || c == '#') {
                end = i;
                break;
            }
        }
        String h = rest.substring(0, end);
        return h.startsWith("www.") ? h.substring(4) : h;
    }

    // text of a piece of HTML: tags removed, the common entities decoded, spaces collapsed
    static String clean(String html) {
        StringBuilder b = new StringBuilder();
        boolean inTag = false;
        for (int i = 0; i < html.length(); i++) {
            char c = html.charAt(i);
            if (c == '<') inTag = true;
            else if (c == '>' && inTag) {
                inTag = false;
                b.append(' ');
            } else if (!inTag) b.append(c);
        }
        String s = b.toString().replace("&amp;", "&").replace("&quot;", "\"").replace("&#x27;", "'").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&nbsp;", " ");
        return s.trim().replaceAll("\\s+", " ");
    }

    // DuckDuckGo: every result has a link with the class «result__a» (the address is inside «uddg=») and a «result__snippet»
    public static List<Item> parseDdg(String html) {
        List<Item> out = new ArrayList<>();
        List<Integer> marks = new ArrayList<>();
        int from = 0;
        while (true) {
            int i = html.indexOf("result__a", from);
            if (i < 0) break;
            marks.add(i);
            from = i + 9;
        }
        for (int k = 0; k < marks.size() && out.size() < MAX_LINKS; k++) {
            int m = marks.get(k);
            int tagStart = html.lastIndexOf("<a", m);
            int tagEnd = html.indexOf('>', m);
            if (tagStart < 0 || tagEnd < 0) continue;
            String tag = html.substring(tagStart, tagEnd);
            String href = attr(tag, "href");
            if (href == null) continue;
            href = href.replace("&amp;", "&");
            String target;
            int p = href.indexOf("uddg=");
            if (p >= 0) {
                String v = href.substring(p + 5);
                int amp = v.indexOf('&');
                target = dec(amp >= 0 ? v.substring(0, amp) : v);
            } else if (href.startsWith("//")) target = "https:" + href;
            else target = href;
            String url = safeUrl(target);
            if (url == null || hostOf(url).endsWith("duckduckgo.com")) continue;
            int close = html.indexOf("</a>", tagEnd);
            if (close < 0) continue;
            String title = clean(html.substring(tagEnd + 1, close));
            int limit = k + 1 < marks.size() ? marks.get(k + 1) : html.length();
            String snippet = "";
            int sp = html.indexOf("result__snippet", close);
            if (sp >= 0 && sp < limit) {
                int s0 = html.indexOf('>', sp);
                if (s0 >= 0) {
                    s0++;
                    int e = html.indexOf("</a>", s0);
                    if (e < 0) e = html.indexOf("</td>", s0);
                    if (e < 0) e = html.indexOf("</div>", s0);
                    if (e > s0) snippet = clean(html.substring(s0, e));
                }
            }
            boolean dup = false;
            for (Item x : out) if (x.url.equals(url)) dup = true;
            if (title.isEmpty() || dup) continue;
            out.add(new Item(title, url, "", snippet));
        }
        return out;
    }

    private static String attr(String tag, String name) {
        String key = name + "=\"";
        int s = tag.indexOf(key);
        if (s < 0) return null;
        s += key.length();
        int e = tag.indexOf('"', s);
        return e < 0 ? null : tag.substring(s, e);
    }

    // YouTube: the page carries its data as JSON; every video has «"videoRenderer":{"videoId":"…»
    public static List<Item> parseYoutube(String html) {
        List<Item> out = new ArrayList<>();
        String key = "\"videoRenderer\":{\"videoId\":\"";
        List<Integer> marks = new ArrayList<>();
        int from = 0;
        while (true) {
            int i = html.indexOf(key, from);
            if (i < 0) break;
            marks.add(i + key.length());
            from = i + key.length();
        }
        for (int k = 0; k < marks.size() && out.size() < MAX_VIDEOS; k++) {
            int m = marks.get(k);
            int e = m;
            while (e < html.length() && (Character.isLetterOrDigit(html.charAt(e)) || html.charAt(e) == '_' || html.charAt(e) == '-')) e++;
            String id = html.substring(m, e);
            if (id.length() != 11) continue;
            boolean dup = false;
            for (Item x : out) if (x.url.endsWith(id)) dup = true;
            if (dup) continue;
            int end = k + 1 < marks.size() ? marks.get(k + 1) : Math.min(html.length(), m + 6000);
            String chunk = html.substring(m, Math.min(end, html.length()));
            String title = textAfter(chunk, "\"title\":{\"runs\":[{\"text\":\"");
            if (title == null || title.trim().isEmpty()) continue;
            String owner = textAfter(chunk, "\"ownerText\":{\"runs\":[{\"text\":\"");
            out.add(new Item(title, "https://www.youtube.com/watch?v=" + id, "https://i.ytimg.com/vi/" + id + "/mqdefault.jpg", owner == null ? "" : owner));
        }
        return out;
    }

    private static String textAfter(String chunk, String key) {
        int p = chunk.indexOf(key);
        if (p < 0) return null;
        p += key.length();
        int e = p;
        while (e < chunk.length() && !(chunk.charAt(e) == '"' && chunk.charAt(e - 1) != '\\')) e++;
        if (e > chunk.length()) return null;
        return chunk.substring(p, e).replace("\\u0026", "&").replace("\\\"", "\"").replace("\\\\", "\\").replace("\\/", "/");
    }

    // Commons: {"query":{"pages":{"1":{"title":"File:X.jpg","index":1,"imageinfo":[{"mime":"image/jpeg","thumburl":"…","descriptionurl":"…"}]}}}}
    public static List<Item> parseCommons(String json) {
        List<Item> out = new ArrayList<>();
        try {
            JSONObject pages = new JSONObject(json).getJSONObject("query").getJSONObject("pages");
            List<int[]> order = new ArrayList<>();
            List<Item> items = new ArrayList<>();
            JSONArray names = pages.names();
            for (int i = 0; names != null && i < names.length(); i++) {
                JSONObject e = pages.getJSONObject(names.getString(i));
                JSONArray info = e.optJSONArray("imageinfo");
                if (info == null || info.length() == 0) continue;
                JSONObject f = info.getJSONObject(0);
                if (!f.optString("mime", "").startsWith("image/")) continue;
                String thumb = safeUrl(f.optString("thumburl", ""));
                String page = safeUrl(f.optString("descriptionurl", ""));
                if (thumb == null || page == null) continue;
                String name = e.optString("title", "").replaceFirst("^File:", "");
                int dot = name.lastIndexOf('.');
                if (dot > 0) name = name.substring(0, dot);
                items.add(new Item(name.replace('_', ' '), page, thumb, ""));
                order.add(new int[]{e.optInt("index", 0), items.size() - 1});
            }
            java.util.Collections.sort(order, (x, y) -> Integer.compare(x[0], y[0]));
            for (int[] o : order) {
                if (out.size() >= MAX_PHOTOS) break;
                out.add(items.get(o[1]));
            }
        } catch (Exception ignored) {
        }
        return out;
    }

    // Wikipedia search: {"query":{"search":[{"title":"…","snippet":"…"}]}}
    public static List<Item> parseWikiSearch(String json, String lang) {
        List<Item> out = new ArrayList<>();
        try {
            JSONArray list = new JSONObject(json).getJSONObject("query").getJSONArray("search");
            for (int i = 0; i < list.length() && out.size() < MAX_LINKS; i++) {
                JSONObject e = list.getJSONObject(i);
                String title = e.optString("title", "").trim();
                if (title.isEmpty()) continue;
                out.add(new Item(title + " — Википедия", "https://" + lang + ".wikipedia.org/wiki/" + enc(title.replace(' ', '_')), "", clean(e.optString("snippet", ""))));
            }
        } catch (Exception ignored) {
        }
        return out;
    }

    // Wikipedia extract: {"query":{"pages":{"1":{"title":"…","index":1,"extract":"…"}}}}
    public static Item parseExtract(String json, String lang) {
        try {
            JSONObject pages = new JSONObject(json).getJSONObject("query").getJSONObject("pages");
            JSONArray names = pages.names();
            JSONObject best = null;
            int bi = Integer.MAX_VALUE;
            for (int i = 0; names != null && i < names.length(); i++) {
                JSONObject e = pages.getJSONObject(names.getString(i));
                int idx = e.optInt("index", Integer.MAX_VALUE);
                if (best == null || idx < bi) {
                    best = e;
                    bi = idx;
                }
            }
            if (best == null) return null;
            String title = best.optString("title", "").trim();
            String text = best.optString("extract", "").trim().replaceAll("\\s+", " ");
            if (title.isEmpty() || text.length() < 40) return null;
            return new Item(title, "https://" + lang + ".wikipedia.org/wiki/" + enc(title.replace(' ', '_')), "", text);
        } catch (Exception e) {
            return null;
        }
    }

    static String lang(String q) {
        for (int i = 0; i < q.length(); i++) if (q.charAt(i) >= '\u0400' && q.charAt(i) <= '\u04FF') return "ru";
        return "en";
    }

    // ---- the network: only https, short timeouts, a limited size
    static String get(String url) throws Exception {
        if (!url.startsWith("https://")) throw new Exception("только https");
        HttpURLConnection c = (HttpURLConnection) new URL(url).openConnection();
        c.setConnectTimeout(8000);
        c.setReadTimeout(15000);
        c.setRequestProperty("User-Agent", "Mozilla/5.0 (Linux; Android 13) Jarvis");
        c.setRequestProperty("Accept-Language", "ru,en;q=0.8");
        int code = c.getResponseCode();
        if (code / 100 != 2) throw new Exception("сервер ответил кодом " + code);
        InputStream in = c.getInputStream();
        StringBuilder b = new StringBuilder();
        try (BufferedReader r = new BufferedReader(new InputStreamReader(in, StandardCharsets.UTF_8))) {
            char[] buf = new char[8192];
            int n;
            while ((n = r.read(buf)) > 0 && b.length() < 3_000_000) b.append(buf, 0, n);
        } finally {
            c.disconnect();
        }
        return b.toString();
    }

    private static List<Item> links(String q) throws Exception {
        List<Item> hits = new ArrayList<>();
        String err = null;
        try {
            hits = parseDdg(get("https://html.duckduckgo.com/html/?q=" + enc(q) + "&kl=ru-ru"));
        } catch (Exception e) {
            err = e.getMessage();
        }
        if (hits.size() < 3) {
            String lang = lang(q);
            try {
                List<Item> w = parseWikiSearch(get("https://" + lang + ".wikipedia.org/w/api.php?action=query&list=search&srsearch=" + enc(q) + "&format=json&srlimit=" + MAX_LINKS + "&utf8=1"), lang);
                for (Item i : w) {
                    boolean dup = false;
                    for (Item x : hits) if (x.url.equals(i.url)) dup = true;
                    if (!dup && hits.size() < MAX_LINKS) hits.add(i);
                }
            } catch (Exception e) {
                if (err == null) err = e.getMessage();
            }
        }
        if (hits.isEmpty() && err != null) throw new Exception(err);
        return hits;
    }

    private static List<Item> photos(String q) throws Exception {
        return parseCommons(get("https://commons.wikimedia.org/w/api.php?action=query&generator=search&gsrnamespace=6&gsrsearch=" + enc(q) + "&gsrlimit=" + (MAX_PHOTOS + 2)
            + "&prop=imageinfo&iiprop=url%7Cmime&iiurlwidth=480&format=json&utf8=1"));
    }

    private static List<Item> videos(String q) throws Exception {
        return parseYoutube(get("https://www.youtube.com/results?search_query=" + enc(q) + "&hl=ru"));
    }

    private static Item text(String q) throws Exception {
        String lang = lang(q);
        return parseExtract(get("https://" + lang + ".wikipedia.org/w/api.php?action=query&generator=search&gsrsearch=" + enc(q)
            + "&gsrlimit=1&prop=extracts&exintro=1&explaintext=1&exchars=2500&format=json&utf8=1"), lang);
    }

    /** The four searches run at the same time; the call waits at most ~25 seconds. Call it away from the main thread. */
    public static Result search(final String q) {
        final Result r = new Result();
        ExecutorService ex = Executors.newFixedThreadPool(4);
        try {
            Future<List<Item>> fp = ex.submit(new Callable<List<Item>>() { public List<Item> call() throws Exception { return photos(q); } });
            Future<List<Item>> fv = ex.submit(new Callable<List<Item>>() { public List<Item> call() throws Exception { return videos(q); } });
            Future<List<Item>> fl = ex.submit(new Callable<List<Item>>() { public List<Item> call() throws Exception { return links(q); } });
            Future<Item> ft = ex.submit(new Callable<Item>() { public Item call() throws Exception { return text(q); } });
            try {
                r.photos.addAll(fp.get(25, TimeUnit.SECONDS));
                if (r.photos.isEmpty()) r.notes.add("Фото: фотографий не нашлось");
            } catch (Exception e) { r.notes.add("Фото: " + why(e)); }
            try {
                r.videos.addAll(fv.get(25, TimeUnit.SECONDS));
                if (r.videos.isEmpty()) r.notes.add("Видео: YouTube не вернул список видео (страница могла измениться)");
            } catch (Exception e) { r.notes.add("Видео: " + why(e)); }
            try {
                r.links.addAll(fl.get(25, TimeUnit.SECONDS));
                if (r.links.isEmpty()) r.notes.add("Ссылки: ссылок не нашлось");
            } catch (Exception e) { r.notes.add("Ссылки: " + why(e)); }
            try {
                r.text = ft.get(25, TimeUnit.SECONDS);
                if (r.text == null) r.notes.add("Текст: статьи в Википедии не нашлось");
            } catch (Exception e) { r.notes.add("Текст: " + why(e)); }
        } finally {
            ex.shutdownNow();
        }
        return r;
    }

    private static String why(Exception e) {
        Throwable t = e.getCause() != null ? e.getCause() : e;
        String m = t.getMessage();
        if (t instanceof java.net.UnknownHostException || t instanceof java.net.ConnectException) return "нет соединения с интернетом";
        if (t instanceof java.net.SocketTimeoutException || t instanceof java.util.concurrent.TimeoutException) return "сервер не отвечает";
        return m == null || m.isEmpty() ? "не получилось" : m;
    }
}
