package app.jarvis.remote;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import org.junit.Test;

import java.io.File;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.InetAddress;
import java.net.URL;
import java.security.KeyStore;
import java.security.cert.CertificateException;
import java.security.cert.X509Certificate;
import javax.net.ssl.HttpsURLConnection;
import javax.net.ssl.KeyManagerFactory;
import javax.net.ssl.SSLContext;
import javax.net.ssl.SSLHandshakeException;
import javax.net.ssl.SSLServerSocket;
import javax.net.ssl.SSLSocket;
import javax.net.ssl.TrustManager;
import javax.net.ssl.X509TrustManager;

import app.jarvis.remote.PinPolicy.Verdict;

public class PinPolicyTest {
    private static final long NOW = System.currentTimeMillis();

    // ---- the decision itself, on real certificates ----

    @Test
    public void theCorrectKeyInsideItsTermIsAccepted() throws Exception {
        TestCerts c = new TestCerts();
        X509Certificate a = c.cert(c.newKey("a", "-1d", 30));
        assertEquals(Verdict.ACCEPT, PinPolicy.evaluate(a, Pin.of(a), null, NOW, false));
        assertTrue(PinPolicy.accepts(Verdict.ACCEPT));
    }

    @Test
    public void anUntrustedKeyNobodyPinnedIsNeverAcceptedQuietly() throws Exception {
        TestCerts c = new TestCerts();
        X509Certificate a = c.cert(c.newKey("a", "-1d", 30));
        // nothing pinned, no link: the owner must compare the fingerprint FIRST (the application sends nothing before that)
        assertEquals(Verdict.ASK_USER, PinPolicy.evaluate(a, null, null, NOW, false));
        assertFalse(PinPolicy.accepts(Verdict.ASK_USER));
    }

    @Test
    public void aServerThatIsNotTheComputerIsRefusedWhateverItsNameOrAddress() throws Exception {
        TestCerts c = new TestCerts();
        X509Certificate real = c.cert(c.newKey("real", "-1d", 30));
        // the attacker has the same address, the same name and a valid term: only the key differs
        X509Certificate fake = c.cert(c.newKey("fake", "-1d", 30));
        assertEquals(Verdict.MISMATCH, PinPolicy.evaluate(fake, Pin.of(real), null, NOW, false));
        // …also when the pin came from the pairing link and nothing is stored yet
        assertEquals(Verdict.MISMATCH, PinPolicy.evaluate(fake, null, Pin.of(real), NOW, false));
        assertFalse(PinPolicy.accepts(Verdict.MISMATCH));
    }

    @Test
    public void thePairingLinkPinIsAcceptedOnceAndRemembered() throws Exception {
        TestCerts c = new TestCerts();
        X509Certificate a = c.cert(c.newKey("a", "-1d", 30));
        assertEquals(Verdict.ACCEPT_AND_REMEMBER, PinPolicy.evaluate(a, null, Pin.of(a), NOW, false));
        // once it is stored the link is not needed (and a link with another key cannot replace the stored one)
        X509Certificate fake = c.cert(c.newKey("fake", "-1d", 30));
        assertEquals(Verdict.MISMATCH, PinPolicy.evaluate(a, Pin.of(fake), Pin.of(a), NOW, false));
    }

    @Test
    public void anExpiredAndANotYetValidCertificateAreRefusedEvenWithTheRightKey() throws Exception {
        TestCerts c = new TestCerts();
        X509Certificate expired = c.cert(c.newKey("old", "-10d", 1));
        X509Certificate future = c.cert(c.newKey("new", "+5d", 30));
        assertEquals(Verdict.EXPIRED, PinPolicy.evaluate(expired, Pin.of(expired), null, NOW, false));
        assertEquals(Verdict.NOT_YET_VALID, PinPolicy.evaluate(future, Pin.of(future), null, NOW, false));
        // the same expired certificate is not rescued by a pairing link either
        assertEquals(Verdict.EXPIRED, PinPolicy.evaluate(expired, null, Pin.of(expired), NOW, false));
        assertFalse(PinPolicy.accepts(Verdict.EXPIRED) || PinPolicy.accepts(Verdict.NOT_YET_VALID));
        // a phone whose clock is far behind meets «not yet valid» and is told to fix the date, not to «proceed»
        X509Certificate a = c.cert(c.newKey("a", "-1d", 30));
        assertEquals(Verdict.NOT_YET_VALID, PinPolicy.evaluate(a, Pin.of(a), null, NOW - 40L * 24 * 3600 * 1000, false));
    }

    @Test
    public void aNewCertificateWithTheSameKeyIsAcceptedAfterAnAddressChange() throws Exception {
        TestCerts c = new TestCerts();
        File store = c.newKey("pc", "-1d", 30);
        String pin = Pin.of(c.cert(store));
        c.reissue(store, "CN=Jarvis Remote", 700);
        X509Certificate again = c.cert(store);
        assertEquals("the PC got a new address and issued the certificate again with its key", Verdict.ACCEPT, PinPolicy.evaluate(again, pin, null, NOW, false));
    }

    @Test
    public void aChangedKeyIsRefusedUntilTheOwnerForgetsTheOldOneAndComparesTheFingerprint() throws Exception {
        TestCerts c = new TestCerts();
        X509Certificate oldKey = c.cert(c.newKey("old", "-1d", 30));
        X509Certificate newKey = c.cert(c.newKey("new", "-1d", 30));
        String stored = Pin.of(oldKey);
        assertEquals(Verdict.MISMATCH, PinPolicy.evaluate(newKey, stored, null, NOW, false));
        // «forget this computer» clears the stored pin: the new key goes through the confirmation again, it is not taken quietly
        assertEquals(Verdict.ASK_USER, PinPolicy.evaluate(newKey, null, null, NOW, false));
        assertFalse(Pin.groups(oldKey).equals(Pin.groups(newKey)));
    }

    @Test
    public void aBrokenCertificateIsNotUsable() throws Exception {
        TestCerts c = new TestCerts();
        X509Certificate a = c.cert(c.newKey("a", "-1d", 30));
        assertEquals(Verdict.UNUSABLE, PinPolicy.evaluate(null, Pin.of(a), null, NOW, false));
        assertEquals(Verdict.UNUSABLE, PinPolicy.evaluate(a, Pin.of(a), null, NOW, true));
        for (Verdict v : Verdict.values()) {
            if (!PinPolicy.accepts(v)) assertFalse("a refusal must say something: " + v, v == Verdict.ASK_USER ? false : PinPolicy.refusal(v).isEmpty());
        }
    }

    // ---- the same decision on a REAL TLS handshake (a local https server with the certificate of the test) ----

    /** The trust manager of the tests: the decision of the application and nothing else (no «trust all»). */
    private static final class PinTrust implements X509TrustManager {
        final String stored;
        final String expected;
        volatile Verdict last;

        PinTrust(String stored, String expected) {
            this.stored = stored;
            this.expected = expected;
        }

        @Override
        public void checkServerTrusted(X509Certificate[] chain, String authType) throws CertificateException {
            last = PinPolicy.evaluate(chain[0], stored, expected, System.currentTimeMillis(), false);
            if (!PinPolicy.accepts(last)) throw new CertificateException(last.name());
        }

        @Override
        public void checkClientTrusted(X509Certificate[] chain, String authType) throws CertificateException {
            throw new CertificateException("no client certificates");
        }

        @Override
        public X509Certificate[] getAcceptedIssuers() {
            return new X509Certificate[0];
        }
    }

    /** A small TLS server on a plain SSLServerSocket (the unit tests of Android do not have the JDK http server): answers «jarvis» to any request. */
    private static final class HttpsServer {
        final SSLServerSocket socket;
        final Thread thread;

        HttpsServer(SSLServerSocket socket) {
            this.socket = socket;
            this.thread = new Thread(() -> {
                while (!socket.isClosed()) {
                    try (SSLSocket client = (SSLSocket) socket.accept()) {
                        client.setSoTimeout(5000);
                        client.startHandshake();
                        byte[] buf = new byte[2048];
                        client.getInputStream().read(buf);
                        byte[] body = "jarvis".getBytes();
                        OutputStream out = client.getOutputStream();
                        out.write(("HTTP/1.1 200 OK\r\nContent-Length: " + body.length + "\r\nConnection: close\r\n\r\n").getBytes());
                        out.write(body);
                        out.flush();
                    } catch (Exception ignored) {
                        // (a refused handshake of a test ends here)
                    }
                }
            });
            this.thread.setDaemon(true);
            this.thread.start();
        }

        int port() {
            return socket.getLocalPort();
        }

        void stop(int ignored) throws Exception {
            socket.close();
        }
    }

    private static HttpsServer serve(TestCerts c, File store) throws Exception {
        KeyStore ks = c.load(store);
        KeyManagerFactory kmf = KeyManagerFactory.getInstance(KeyManagerFactory.getDefaultAlgorithm());
        kmf.init(ks, TestCerts.PASS.toCharArray());
        SSLContext ctx = SSLContext.getInstance("TLS");
        ctx.init(kmf.getKeyManagers(), null, null);
        SSLServerSocket socket = (SSLServerSocket) ctx.getServerSocketFactory().createServerSocket(0, 5, InetAddress.getLoopbackAddress());
        return new HttpsServer(socket);
    }

    private static String get(HttpsServer server, PinTrust trust) throws Exception {
        SSLContext ctx = SSLContext.getInstance("TLS");
        ctx.init(null, new TrustManager[]{trust}, null);
        HttpsURLConnection con = (HttpsURLConnection) new URL("https://127.0.0.1:" + server.port() + "/").openConnection();
        con.setSSLSocketFactory(ctx.getSocketFactory());
        // the NAME is not what is trusted (the pin is): the address of the computer may change
        con.setHostnameVerifier((h, s) -> true);
        con.setConnectTimeout(5000);
        con.setReadTimeout(5000);
        try (InputStream in = con.getInputStream()) {
            return new String(in.readAllBytes());
        }
    }

    private static void refused(HttpsServer server, PinTrust trust, Verdict expected) throws Exception {
        try {
            get(server, trust);
            fail("the handshake must be refused: " + expected);
        } catch (SSLHandshakeException e) {
            assertEquals(expected, trust.last);
        }
    }

    @Test
    public void aRealHandshakeWithTheRightKeyWorksAndWithAnotherKeyAnExpiredOrAFutureCertificateItFails() throws Exception {
        TestCerts c = new TestCerts();
        File real = c.newKey("real", "-1d", 30);
        String pin = Pin.of(c.cert(real));
        HttpsServer ok = serve(c, real);
        try {
            assertEquals("jarvis", get(ok, new PinTrust(pin, null)));
            // the first pairing by the link: accepted because the key is the one in the link
            PinTrust viaLink = new PinTrust(null, pin);
            assertEquals("jarvis", get(ok, viaLink));
            assertEquals(Verdict.ACCEPT_AND_REMEMBER, viaLink.last);
            // nothing pinned, no link: refused (the application asks the owner instead; no data goes over this connection)
            refused(ok, new PinTrust(null, null), Verdict.ASK_USER);
        } finally {
            ok.stop(0);
        }
        // substitution: another server answers on the same address
        HttpsServer fake = serve(c, c.newKey("fake", "-1d", 30));
        try {
            refused(fake, new PinTrust(pin, null), Verdict.MISMATCH);
        } finally {
            fake.stop(0);
        }
        // expired and not yet valid, with the right key
        File old = c.newKey("old", "-10d", 1);
        HttpsServer expired = serve(c, old);
        try {
            refused(expired, new PinTrust(Pin.of(c.cert(old)), null), Verdict.EXPIRED);
        } finally {
            expired.stop(0);
        }
        File future = c.newKey("future", "+5d", 30);
        HttpsServer notYet = serve(c, future);
        try {
            refused(notYet, new PinTrust(Pin.of(c.cert(future)), null), Verdict.NOT_YET_VALID);
        } finally {
            notYet.stop(0);
        }
    }

    @Test
    public void aRealHandshakeAfterTheAddressChangedStillWorksBecauseTheKeyIsTheSame() throws Exception {
        TestCerts c = new TestCerts();
        File store = c.newKey("pc", "-1d", 30);
        String pin = Pin.of(c.cert(store));
        c.reissue(store, "CN=Jarvis Remote", 700);
        HttpsServer server = serve(c, store);
        try {
            assertEquals("jarvis", get(server, new PinTrust(pin, null)));
        } finally {
            server.stop(0);
        }
    }
}
