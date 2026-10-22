package app.jarvis.remote;

import java.io.File;
import java.io.FileInputStream;
import java.nio.file.Files;
import java.security.KeyStore;
import java.security.cert.X509Certificate;
import java.util.concurrent.TimeUnit;

/** Makes real certificates with the keytool of the JDK that runs the tests (no third-party library, nothing is mocked). */
final class TestCerts {
    static final String PASS = "changeit";
    final File dir;

    TestCerts() throws Exception {
        dir = Files.createTempDirectory("jarvis-pin-test").toFile();
        dir.deleteOnExit();
    }

    private static void run(String... args) throws Exception {
        String keytool = System.getProperty("java.home") + File.separator + "bin" + File.separator + "keytool";
        String[] cmd = new String[args.length + 1];
        cmd[0] = keytool;
        System.arraycopy(args, 0, cmd, 1, args.length);
        ProcessBuilder pb = new ProcessBuilder(cmd).redirectErrorStream(true);
        pb.environment().remove("JAVA_TOOL_OPTIONS");
        Process p = pb.start();
        String out = new String(p.getInputStream().readAllBytes());
        if (!p.waitFor(60, TimeUnit.SECONDS) || p.exitValue() != 0) throw new IllegalStateException("keytool failed: " + out);
    }

    /** A new key and a self-signed certificate for 127.0.0.1; `startOffset` like "-10d" / "+5d", validity in days from the start. */
    File newKey(String name, String startOffset, int validityDays) throws Exception {
        File f = new File(dir, name + ".p12");
        run("-genkeypair", "-keystore", f.getPath(), "-storetype", "PKCS12", "-storepass", PASS, "-keypass", PASS, "-alias", "a",
                "-keyalg", "EC", "-groupname", "secp256r1", "-dname", "CN=Jarvis Remote", "-ext", "san=ip:127.0.0.1,dns:localhost",
                "-startdate", startOffset, "-validity", String.valueOf(validityDays));
        return f;
    }

    /** The same key, a NEW certificate (the PC issues the certificate again with its key when its address changes). */
    void reissue(File store, String dname, int validityDays) throws Exception {
        run("-selfcert", "-keystore", store.getPath(), "-storetype", "PKCS12", "-storepass", PASS, "-alias", "a", "-dname", dname, "-validity", String.valueOf(validityDays),
                "-startdate", "-1d", "-ext", "san=ip:10.0.0.7,dns:localhost");
    }

    KeyStore load(File f) throws Exception {
        KeyStore ks = KeyStore.getInstance("PKCS12");
        try (FileInputStream in = new FileInputStream(f)) {
            ks.load(in, PASS.toCharArray());
        }
        return ks;
    }

    X509Certificate cert(File f) throws Exception {
        return (X509Certificate) load(f).getCertificate("a");
    }
}
