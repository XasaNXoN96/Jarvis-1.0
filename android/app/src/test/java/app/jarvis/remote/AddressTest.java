package app.jarvis.remote;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNull;

import org.junit.Test;

public class AddressTest {
    @Test
    public void anIpOrANameGetsHttpsAndTheDefaultPort() {
        assertEquals("https://192.168.1.10:9713/", Address.normalize("192.168.1.10"));
        assertEquals("https://192.168.1.10:9713/", Address.normalize("  192.168.1.10  "));
        assertEquals("https://my-pc.local:9713/", Address.normalize("my-pc.local"));
    }

    @Test
    public void aPortIsKeptAndPlainHttpIsNeverAnAddress() {
        assertEquals("https://192.168.1.10:8080/", Address.normalize("192.168.1.10:8080"));
        assertEquals("https://192.168.1.10:9713/", Address.normalize("https://192.168.1.10:9713/"));
        // the phone never talks to the computer without encryption: no hidden fallback to http
        assertNull(Address.normalize("http://192.168.1.10:9713/"));
        assertNull(Address.normalize("http://192.168.1.10:9713/?t=abc123"));
        assertNull(Address.normalize("HTTP://192.168.1.10"));
    }

    @Test
    public void theSecretOfTheLinkOfATunnelIsKeptAndEncodedAgain() {
        assertEquals("https://x.trycloudflare.com:9713/?t=abc123", Address.normalize("https://x.trycloudflare.com:9713/?t=abc123"));
        assertEquals("https://192.168.1.10:9713/?t=a%26b%3Dc", Address.normalize("https://192.168.1.10:9713/?t=a%26b%3Dc&x=1"));
        assertEquals("https://192.168.1.10:9713/", Address.normalize("https://192.168.1.10:9713/?x=1"));
    }

    private static final String PIN = "sha256/AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-_AbCde";

    @Test
    public void aPairingLinkGivesTheAddressThePinAndTheCode() {
        Address.Parsed p = Address.parse("https://192.168.1.10:9713/#pin=" + PIN + "&c=123456");
        assertEquals("https://192.168.1.10:9713/", p.url);
        assertEquals("192.168.1.10", p.host);
        assertEquals(PIN, p.pin);
        assertEquals("123456", p.code);
        // the pin alone (no code shown on the computer yet)
        Address.Parsed q = Address.parse("https://192.168.1.10:9713/#pin=" + PIN);
        assertEquals(PIN, q.pin);
        assertNull(q.code);
        // a plain address carries neither
        Address.Parsed r = Address.parse("192.168.1.10");
        assertNull(r.pin);
        assertNull(r.code);
        // the pin and the code never end up in the saved link
        assertEquals("https://192.168.1.10:9713/", Address.normalize("https://192.168.1.10:9713/#pin=" + PIN + "&c=123456"));
    }

    @Test
    public void aDamagedPinOrAWrongCodeIsNotQuietlyDropped() {
        // a damaged pin means a damaged link: refused, never «no pin» (that would open the first-use path without a check)
        assertNull(Address.parse("https://192.168.1.10:9713/#pin=sha256/short"));
        assertNull(Address.parse("https://192.168.1.10:9713/#pin=md5/AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-_AbCde"));
        // a code that is not six digits is ignored (the owner types it), the pin stays
        Address.Parsed p = Address.parse("https://192.168.1.10:9713/#pin=" + PIN + "&c=12ab");
        assertEquals(PIN, p.pin);
        assertNull(p.code);
    }

    @Test
    public void notAnAddressIsRefused() {
        for (String bad : new String[]{null, "", "   ", "not an address", "javascript:alert(1)", "file:///sdcard/x", "https://user:pw@192.168.1.10/", "999.1.1.1", "192.168.1.10/../x y", "https://", "ftp://example.com", "content://x"}) {
            assertNull("must be refused: " + bad, Address.normalize(bad));
        }
        StringBuilder longText = new StringBuilder("https://");
        for (int i = 0; i < 500; i++) longText.append('a');
        assertNull(Address.normalize(longText.toString()));
    }

    @Test
    public void aHostCannotCarryAPathOrAScript() {
        assertNull(Address.normalize("https://exa mple.com/"));
        assertNull(Address.normalize("https://evil.com\\@192.168.1.10/"));
        assertEquals("192.168.1.10", Address.hostOf("https://192.168.1.10:9713/"));
        assertEquals("", Address.hostOf("garbage"));
    }
}
