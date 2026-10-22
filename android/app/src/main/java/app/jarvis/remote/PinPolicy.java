package app.jarvis.remote;

import java.security.cert.X509Certificate;

/**
 * The ONE decision about the certificate of the computer: whether the phone trusts it. There is no «proceed anyway»:
 * a certificate is accepted only if its public key is the pinned one (or the one named in the pairing link that the owner
 * scanned/typed, or the one the owner confirmed by comparing the fingerprint), and only inside its term.
 * Pure Java (the same function runs in the application and in the tests on a real TLS handshake).
 */
final class PinPolicy {
    enum Verdict {
        /** the key is the pinned one and the term is valid */
        ACCEPT,
        /** the key is the one named in the pairing link: accepted, and pinned from now on */
        ACCEPT_AND_REMEMBER,
        /** nothing is pinned yet and no link named a key: the owner must compare the fingerprint first (nothing is sent before that) */
        ASK_USER,
        /** another key than the pinned/expected one: another computer, a man in the middle, or the key of the computer was changed */
        MISMATCH,
        EXPIRED,
        NOT_YET_VALID,
        /** no usable certificate, or the system reports it as broken */
        UNUSABLE
    }

    private PinPolicy() {
    }

    /**
     * @param stored   the pin kept by the application ("sha256/…") or null
     * @param expected the pin from a pairing link that was just opened, or null
     * @param invalid  the system reported an error that is not «untrusted» or «name mismatch» (a damaged certificate)
     */
    static Verdict evaluate(X509Certificate leaf, String stored, String expected, long nowMillis, boolean invalid) {
        if (leaf == null || invalid) return Verdict.UNUSABLE;
        String presented;
        try {
            presented = Pin.of(leaf);
        } catch (Exception e) {
            return Verdict.UNUSABLE;
        }
        if (stored != null && !Pin.same(stored, presented)) return Verdict.MISMATCH;
        if (stored == null && expected != null && !Pin.same(expected, presented)) return Verdict.MISMATCH;
        if (nowMillis < leaf.getNotBefore().getTime()) return Verdict.NOT_YET_VALID;
        if (nowMillis > leaf.getNotAfter().getTime()) return Verdict.EXPIRED;
        if (stored != null) return Verdict.ACCEPT;
        if (expected != null) return Verdict.ACCEPT_AND_REMEMBER;
        return Verdict.ASK_USER;
    }

    static boolean accepts(Verdict v) {
        return v == Verdict.ACCEPT || v == Verdict.ACCEPT_AND_REMEMBER;
    }

    /** What the owner is told when the computer is refused. */
    static String refusal(Verdict v) {
        switch (v) {
            case MISMATCH:
                return "Это не тот компьютер, который вы привязывали, или ключ компьютера был заменён. Если вы сами сменили ключ в Jarvis — привяжите телефон заново. Если нет — возможно, в вашей сети подмена: не вводите код.";
            case EXPIRED:
                return "Срок сертификата компьютера истёк. Перезапустите Jarvis на компьютере: он выпустит новый сертификат с тем же ключом. Проверьте и дату на компьютере.";
            case NOT_YET_VALID:
                return "Сертификат компьютера ещё не действует: проверьте дату и время на телефоне и на компьютере.";
            default:
                return "Сертификат компьютера повреждён или недоступен. Соединение не открыто.";
        }
    }
}
