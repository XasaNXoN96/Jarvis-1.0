package app.jarvis.remote;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNull;

import org.junit.Test;

public class AddressTest {
    @Test
    public void anIpOrANameGetsTheDefaultPort() {
        assertEquals("http://192.168.1.10:9713/", Address.normalize("192.168.1.10"));
        assertEquals("http://192.168.1.10:9713/", Address.normalize("  192.168.1.10  "));
        assertEquals("http://my-pc.local:9713/", Address.normalize("my-pc.local"));
    }

    @Test
    public void aPortAndTheSchemeAreKept() {
        assertEquals("http://192.168.1.10:8080/", Address.normalize("192.168.1.10:8080"));
        assertEquals("https://192.168.1.10:9713/", Address.normalize("https://192.168.1.10:9713/"));
    }

    @Test
    public void theSecretOfTheLinkIsKeptAndEncodedAgain() {
        assertEquals("http://192.168.1.10:9713/?t=abc123", Address.normalize("http://192.168.1.10:9713/?t=abc123"));
        assertEquals("http://192.168.1.10:9713/?t=a%26b%3Dc", Address.normalize("http://192.168.1.10:9713/?t=a%26b%3Dc&x=1"));
        assertEquals("http://192.168.1.10:9713/", Address.normalize("http://192.168.1.10:9713/?x=1"));
    }

    @Test
    public void notAnAddressIsRefused() {
        for (String bad : new String[]{null, "", "   ", "not an address", "javascript:alert(1)", "file:///sdcard/x", "http://user:pw@192.168.1.10/", "999.1.1.1", "192.168.1.10/../x y", "http://", "ftp://example.com"}) {
            assertNull("must be refused: " + bad, Address.normalize(bad));
        }
        StringBuilder longText = new StringBuilder("http://");
        for (int i = 0; i < 400; i++) longText.append('a');
        assertNull(Address.normalize(longText.toString()));
    }

    @Test
    public void aHostCannotCarryAPathOrAScript() {
        assertNull(Address.normalize("http://exa mple.com/"));
        assertNull(Address.normalize("http://evil.com\\@192.168.1.10/"));
        assertEquals("192.168.1.10", Address.hostOf("http://192.168.1.10:9713/"));
        assertEquals("", Address.hostOf("garbage"));
    }
}
