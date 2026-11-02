package app.jarvis.remote.phone;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import java.util.List;
import org.junit.Test;

public class WebSearchTest {
    private static final String PAGE = "<div class=\"result results_links\"><h2 class=\"result__title\"><a rel=\"nofollow\" class=\"result__a\" href=\"//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fborsch%3Fa%3D1&amp;rut=abc\">Рецепт <b>борща</b> &amp; свёкла</a></h2>\n"
        + "<a class=\"result__snippet\" href=\"//duckduckgo.com/l/?uddg=x\">Как сварить <b>борщ</b> за час: свёкла, капуста &quot;и мясо&quot;.</a></div>\n"
        + "<div class=\"result\"><a class=\"result__a\" href=\"https://ads.duckduckgo.com/y.js?ad=1\">Реклама</a></div>\n"
        + "<div class=\"result\"><a rel=\"nofollow\" class=\"result__a\" href=\"//duckduckgo.com/l/?uddg=http%3A%2F%2Fuser%3Apass%40bad.example%2F&amp;rut=1\">С паролем в адресе</a></div>\n"
        + "<div class=\"result\"><a class=\"result__a\" href=\"//duckduckgo.com/l/?uddg=javascript%3Aalert(1)\">Скрипт</a></div>\n"
        + "<div class=\"result\"><a class=\"result__a\" href=\"//duckduckgo.com/l/?uddg=https%3A%2F%2Fsecond.org%2F\">Второй сайт</a><a class=\"result__snippet\" href=\"x\">Второй текст</a></div>\n"
        + "<div class=\"result\"><a class=\"result__a\" href=\"//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fborsch%3Fa%3D1\">Тот же адрес</a></div>";

    @Test public void duckduckgo_results_are_read_cleaned_and_filtered() {
        List<WebSearch.Item> h = WebSearch.parseDdg(PAGE);
        assertEquals(2, h.size());
        assertEquals("Рецепт борща & свёкла", h.get(0).title);
        assertEquals("https://example.com/borsch?a=1", h.get(0).url);
        assertEquals("Как сварить борщ за час: свёкла, капуста \"и мясо\".", h.get(0).note);
        assertEquals("https://second.org/", h.get(1).url);
        assertEquals("Второй текст", h.get(1).note);
        assertTrue(WebSearch.parseDdg("").isEmpty() && WebSearch.parseDdg("<html>ничего</html>").isEmpty());
        for (String broken : new String[]{"result__a", "<a class=\"result__a\"", "<a class=\"result__a\" href=\"", "<a class=\"result__a\" href=\"//x\">без конца", "%"}) {
            WebSearch.parseDdg(broken);
        }
    }

    @Test public void youtube_videos_are_read_from_the_page_data() {
        String page = "var ytInitialData = {\"contents\":[{\"videoRenderer\":{\"videoId\":\"abcDEF12345\",\"thumbnail\":{},\"title\":{\"runs\":[{\"text\":\"Zoom \\\"Market\\\" \\u0026 обзор\"}],\"accessibility\":{}},\"ownerText\":{\"runs\":[{\"text\":\"Канал Один\"}]}}},\n"
            + "{\"videoRenderer\":{\"videoId\":\"zzzzzzzzzzz\",\"title\":{\"runs\":[{\"text\":\"Второе видео\"}]},\"ownerText\":{\"runs\":[{\"text\":\"Канал Два\"}]}}},\n"
            + "{\"videoRenderer\":{\"videoId\":\"abcDEF12345\",\"title\":{\"runs\":[{\"text\":\"Дубль\"}]}}},\n"
            + "{\"videoRenderer\":{\"videoId\":\"short\",\"title\":{\"runs\":[{\"text\":\"Не видео\"}]}}}]}";
        List<WebSearch.Item> v = WebSearch.parseYoutube(page);
        assertEquals(2, v.size());
        assertEquals("Zoom \"Market\" & обзор", v.get(0).title);
        assertEquals("https://www.youtube.com/watch?v=abcDEF12345", v.get(0).url);
        assertEquals("Канал Один", v.get(0).note);
        assertEquals("https://i.ytimg.com/vi/abcDEF12345/mqdefault.jpg", v.get(0).thumb);
        assertTrue(WebSearch.parseYoutube("").isEmpty());
    }

    @Test public void only_safe_addresses_are_kept() {
        assertTrue(WebSearch.safeUrl("https://example.com/a?b=c") != null);
        for (String bad : new String[]{"javascript:alert(1)", "file:///c:/x", "https://user:pw@example.com/", "https://localhost/", "https:///x", "https://exa mple.com/", "ftp://example.com/", ""}) {
            assertTrue(bad, WebSearch.safeUrl(bad) == null);
        }
        assertEquals("example.com", WebSearch.hostOf("https://www.example.com/a?b"));
        assertEquals("ru", WebSearch.lang("рецепт борща"));
        assertEquals("en", WebSearch.lang("rust language"));
    }
}
