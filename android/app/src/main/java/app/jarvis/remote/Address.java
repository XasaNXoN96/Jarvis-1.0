package app.jarvis.remote;

import java.net.URI;
import java.net.URLDecoder;
import java.net.URLEncoder;
import java.nio.charset.StandardCharsets;
import java.util.regex.Pattern;

/** The address of the computer that the user typed: only a host (a name or an IP), a port and the secret of the link. Pure Java, so it is tested without a phone. */
final class Address {
    static final int PORT = 9713;
    private static final Pattern HOST = Pattern.compile("[A-Za-z0-9._-]+");
    private static final Pattern IPV4 = Pattern.compile("(\\d{1,3})\\.(\\d{1,3})\\.(\\d{1,3})\\.(\\d{1,3})");

    private Address() {
    }

    /** "192.168.1.10", "192.168.1.10:9713" or a whole link (with ?t=…) -> the link to the page of the remote; null when it is not an address. */
    static String normalize(String raw) {
        String s = raw == null ? "" : raw.trim();
        if (s.isEmpty() || s.length() > 300 || s.matches(".*\\s.*")) return null;
        // another scheme (file:, ftp:, content:, javascript:) is never an address of the computer
        if (s.contains("://") && !s.startsWith("http://") && !s.startsWith("https://")) return null;
        if (!s.startsWith("http://") && !s.startsWith("https://")) s = "http://" + s;
        URI u;
        try {
            u = new URI(s);
        } catch (Exception e) {
            return null;
        }
        String h = u.getHost();
        if (h == null || h.isEmpty() || !HOST.matcher(h).matches()) return null;
        // a number that looks like an IP address must be one (999.1.1.1 is not an address)
        java.util.regex.Matcher m = IPV4.matcher(h);
        if (m.matches()) {
            for (int i = 1; i <= 4; i++) if (Integer.parseInt(m.group(i)) > 255) return null;
        }
        if (u.getUserInfo() != null) return null; // a link with a login and a password is never taken
        int port = u.getPort() > 0 && u.getPort() < 65536 ? u.getPort() : PORT;
        String scheme = "https".equals(u.getScheme()) ? "https" : "http";
        String t = queryParam(u.getRawQuery(), "t");
        String tail = t == null || t.isEmpty() || t.length() > 200 ? "" : "?t=" + URLEncoder.encode(t, StandardCharsets.UTF_8);
        return scheme + "://" + h + ":" + port + "/" + tail;
    }

    static String queryParam(String query, String name) {
        if (query == null) return null;
        for (String kv : query.split("&")) {
            int i = kv.indexOf('=');
            if (i > 0 && kv.substring(0, i).equals(name)) {
                try {
                    return URLDecoder.decode(kv.substring(i + 1), StandardCharsets.UTF_8);
                } catch (Exception e) {
                    return null;
                }
            }
        }
        return null;
    }

    /** The host of a saved link, "" when it is not one. */
    static String hostOf(String url) {
        try {
            String h = new URI(url).getHost();
            return h == null ? "" : h;
        } catch (Exception e) {
            return "";
        }
    }
}
