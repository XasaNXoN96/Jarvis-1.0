package app.jarvis.remote;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

import java.io.File;

public class PinTest {
    @Test
    public void thePinIsTheHashOfThePublicKeyAndHasTheAgreedForms() throws Exception {
        TestCerts c = new TestCerts();
        File a = c.newKey("a", "-1d", 30);
        String pin = Pin.of(c.cert(a));
        assertTrue(pin, Pin.valid(pin));
        assertEquals(7 + 43, pin.length());
        assertEquals(pin, Pin.ofKey(c.cert(a).getPublicKey().getEncoded()));
        String g = Pin.groups(c.cert(a));
        assertTrue(g, g.matches("([0-9A-F]{4}-){5}[0-9A-F]{4}"));
    }

    @Test
    public void theSameKeyKeepsThePinWhateverTheCertificateSaysAndAnotherKeyChangesIt() throws Exception {
        TestCerts c = new TestCerts();
        File a = c.newKey("a", "-1d", 30);
        String before = Pin.of(c.cert(a));
        String groupsBefore = Pin.groups(c.cert(a));
        c.reissue(a, "CN=Jarvis Remote, O=new address", 700);
        assertEquals("a new certificate (new address, new term) with the same key: the same pin", before, Pin.of(c.cert(a)));
        assertEquals(groupsBefore, Pin.groups(c.cert(a)));
        File other = c.newKey("other", "-1d", 30);
        assertFalse("another key: another pin", Pin.same(before, Pin.of(c.cert(other))));
    }

    @Test
    public void onlyTheAgreedFormIsAPin() {
        assertFalse(Pin.valid(null));
        assertFalse(Pin.valid(""));
        assertFalse(Pin.valid("sha256/short"));
        assertFalse(Pin.valid("sha1/AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-_AbCde"));
        assertFalse(Pin.valid("sha256/AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-_AbCde="));
        assertTrue(Pin.valid("sha256/AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-_AbCde"));
        assertFalse(Pin.same(null, "x"));
        assertFalse(Pin.same("x", null));
        assertTrue(Pin.same("abc", "abc"));
        assertFalse(Pin.same("abc", "abd"));
    }
}
