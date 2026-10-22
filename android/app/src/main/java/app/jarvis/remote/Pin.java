package app.jarvis.remote;

import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.cert.X509Certificate;
import java.util.Base64;
import java.util.regex.Pattern;

/**
 * The pin of the computer: the SHA-256 of the public key of its certificate (SubjectPublicKeyInfo), written "sha256/" + base64url without padding.
 * The key of the computer does not change when its address or its certificate does, so the pin survives a new IP address and a renewal.
 * Pure Java: tested without a phone.
 */
final class Pin {
    private static final Pattern FORM = Pattern.compile("sha256/[A-Za-z0-9_-]{43}");

    private Pin() {
    }

    static boolean valid(String pin) {
        return pin != null && FORM.matcher(pin).matches();
    }

    /** "sha256/…" of the public key of this certificate. */
    static String of(X509Certificate cert) {
        return ofKey(cert.getPublicKey().getEncoded());
    }

    static String ofKey(byte[] spki) {
        return "sha256/" + Base64.getUrlEncoder().withoutPadding().encodeToString(sha256(spki));
    }

    /** The same hash for the eyes: 24 hex characters in groups of four («A1B2-C3D4-…»), as the window of the computer shows it. */
    static String groups(X509Certificate cert) {
        byte[] h = sha256(cert.getPublicKey().getEncoded());
        StringBuilder hex = new StringBuilder();
        for (byte b : h) hex.append(String.format("%02X", b & 0xff));
        String s = hex.substring(0, 24);
        StringBuilder out = new StringBuilder();
        for (int i = 0; i < 24; i += 4) {
            if (i > 0) out.append('-');
            out.append(s, i, i + 4);
        }
        return out.toString();
    }

    /** The comparison takes the same time wherever the texts differ. */
    static boolean same(String a, String b) {
        if (a == null || b == null) return false;
        return MessageDigest.isEqual(a.getBytes(StandardCharsets.UTF_8), b.getBytes(StandardCharsets.UTF_8));
    }

    private static byte[] sha256(byte[] data) {
        try {
            return MessageDigest.getInstance("SHA-256").digest(data);
        } catch (Exception e) {
            throw new IllegalStateException(e);
        }
    }
}
