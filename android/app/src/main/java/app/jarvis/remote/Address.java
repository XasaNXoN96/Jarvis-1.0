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

    /** What was typed or scanned: the link of the page, the host, and (from a pairing link) the pin of the computer and the one-time code. */
    static final class Parsed {
        final String url;
        final String host;
        /** "sha256/…" from the «#pin=» part of the link, or null */
        final String pin;
        /** the six digits from «&c=», or null */
        final String code;

        Parsed(String url, String host, String pin, String code) {
            this.url = url;
            this.host = host;
            this.pin = pin;
            this.code = code;
        }
    }

    /**
     * "192.168.1.10", "192.168.1.10:9713", "https://192.168.1.10:9713/" or a whole pairing link
     * ("https://192.168.1.10:9713/#pin=sha256/…&c=123456") -> the link to the page of the remote, https ONLY; null when it is not an address.
     * A link with plain «http://» is refused: the phone never talks to the computer without encryption.
     */
    static Parsed parse(String raw) {
        String s = raw == null ? "" : raw.trim();
        if (s.isEmpty() || s.length() > 400 || s.matches(".*\\s.*")) return null;
        if (s.contains("://")) {
            // plain http and any other scheme (file:, ftp:, content:, javascript:) are never an address of the computer
            if (!s.startsWith("https://")) return null;
        } else {
            s = "https://" + s;
        }
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
        String t = queryParam(u.getRawQuery(), "t");
        String tail = t == null || t.isEmpty() || t.length() > 200 ? "" : "?t=" + URLEncoder.encode(t, StandardCharsets.UTF_8);
        String pin = queryParam(u.getRawFragment(), "pin");
        if (pin != null && !Pin.valid(pin)) return null; // a damaged pin is an error, never «no pin»
        String code = queryParam(u.getRawFragment(), "c");
        if (code != null && !code.matches("\\d{6}")) code = null;
        return new Parsed("https://" + h + ":" + port + "/" + tail, h, pin, code);
    }

    /** The link to the page of the remote (https), or null: see {@link #parse}. */
    static String normalize(String raw) {
        Parsed p = parse(raw);
        return p == null ? null : p.url;
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
