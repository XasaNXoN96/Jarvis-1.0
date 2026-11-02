package app.jarvis.remote.phone;

import java.util.ArrayList;
import java.util.List;

/**
 * Words as people write them: lower case, «ё» = «е», no punctuation; and the slips of a keyboard: a missing, an extra, a wrong letter,
 * two neighbours swapped, a word typed in the other layout («ghbdtn» = «привет»). Pure Java: it is tested without a phone.
 */
public final class Norm {
    private Norm() { }

    public static String normalize(String s) {
        if (s == null) return "";
        StringBuilder b = new StringBuilder();
        boolean space = true;
        for (int i = 0; i < s.length(); i++) {
            char c = Character.toLowerCase(s.charAt(i));
            if (c == 'ё') c = 'е';
            if (Character.isLetterOrDigit(c)) {
                b.append(c);
                space = false;
            } else if (!space) {
                b.append(' ');
                space = true;
            }
        }
        return b.toString().trim();
    }

    public static List<String> words(String s) {
        List<String> out = new ArrayList<>();
        String n = normalize(s);
        if (n.isEmpty()) return out;
        for (String w : n.split(" ")) if (!w.isEmpty()) out.add(w);
        return out;
    }

    // the number of edits where a swap of two neighbouring letters is ONE edit («отрпавь» -> «отправь»)
    public static int typo(String a, String b) {
        int n = a.length(), m = b.length();
        int[][] d = new int[n + 1][m + 1];
        for (int i = 0; i <= n; i++) d[i][0] = i;
        for (int j = 0; j <= m; j++) d[0][j] = j;
        for (int i = 1; i <= n; i++) {
            for (int j = 1; j <= m; j++) {
                int cost = a.charAt(i - 1) == b.charAt(j - 1) ? 0 : 1;
                d[i][j] = Math.min(Math.min(d[i - 1][j] + 1, d[i][j - 1] + 1), d[i - 1][j - 1] + cost);
                if (i > 1 && j > 1 && a.charAt(i - 1) == b.charAt(j - 2) && a.charAt(i - 2) == b.charAt(j - 1)) {
                    d[i][j] = Math.min(d[i][j], d[i - 2][j - 2] + 1);
                }
            }
        }
        return d[n][m];
    }

    // 0.0 .. 1.0
    public static double similarity(String a, String b) {
        a = normalize(a);
        b = normalize(b);
        int n = Math.max(a.length(), b.length());
        if (n == 0) return 1.0;
        return 1.0 - (double) typo(a, b) / n;
    }

    private static final String EN = "qwertyuiop[]asdfghjkl;'zxcvbnm,.`";
    private static final String RU = "йцукенгшщзхъфывапролджэячсмитьбюё";

    public static String enToRu(String s) {
        StringBuilder b = new StringBuilder();
        for (char c : s.toLowerCase().toCharArray()) {
            int i = EN.indexOf(c);
            b.append(i >= 0 ? RU.charAt(i) : c);
        }
        return b.toString();
    }

    public static String ruToEn(String s) {
        StringBuilder b = new StringBuilder();
        for (char c : s.toLowerCase().toCharArray()) {
            int i = RU.indexOf(c);
            b.append(i >= 0 ? EN.charAt(i) : c);
        }
        return b.toString();
    }

    // is the word a slip of one of the known words? a word of 7 letters and more may differ in two, a shorter one (4..6) in one; a word of up to 4 letters
    // must keep its first letter; words of up to 2 letters must be exact. Returns the word that was meant, or null.
    public static String nearWord(String word, String... known) {
        String w = normalize(word);
        if (w.isEmpty()) return null;
        if (w.length() < 3) {
            for (String k : known) if (normalize(k).equals(w)) return k;
            return null;
        }
        String[] variants = {w, normalize(enToRu(w)), normalize(ruToEn(w))};
        String best = null;
        int bestD = Integer.MAX_VALUE;
        for (String k : known) {
            String kn = normalize(k);
            int limit = kn.length() >= 7 ? 2 : (kn.length() >= 4 ? 1 : 0);
            for (String v : variants) {
                if (kn.length() <= 4 && (v.isEmpty() || v.charAt(0) != kn.charAt(0))) continue;
                int d = typo(v, kn);
                if (d <= limit && d < bestD) {
                    best = k;
                    bestD = d;
                }
            }
        }
        return best;
    }

    // «маме» ~ «мама»: the same beginning, the endings may differ
    public static boolean sameStem(String a, String b) {
        a = normalize(a);
        b = normalize(b);
        if (a.equals(b)) return true;
        int common = 0;
        int m = Math.min(a.length(), b.length());
        while (common < m && a.charAt(common) == b.charAt(common)) common++;
        return (common >= 3 && common + 2 >= m) || (common >= 2 && m <= 4 && common + 1 >= m);
    }
}
