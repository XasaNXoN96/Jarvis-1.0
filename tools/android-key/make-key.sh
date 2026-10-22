#!/usr/bin/env bash
# Creates the NEW Android release key on YOUR computer and stores it as the three protected secrets of the repository (GitHub CLI "gh").
# - a new random key and password every run; an existing key is never replaced;
# - the password goes to keytool through an environment variable (never on a command line) and is never printed;
# - the key and the password stay in ~/jarvis-android-key (mode 700/600) — back that folder up: without the key the app cannot be updated;
# - nothing is committed (the folder is outside the repository and ignored by .gitignore).
# Run:  bash tools/android-key/make-key.sh
set -euo pipefail
repo='XasaNXoN96/Jarvis-1.0'
dir="${JARVIS_KEY_DIR:-$HOME/jarvis-android-key}"
command -v keytool >/dev/null 2>&1 || { echo "keytool (a JDK) is needed: install JDK 17 (e.g. Temurin) and run again." >&2; exit 1; }
umask 077
if [ -e "$dir/jarvis-release.jks" ]; then
  # an existing key is never replaced; with --upload it is only (re)sent to the secrets (e.g. after "gh auth login")
  [ "${1:-}" = "--upload" ] && [ -s "$dir/password.txt" ] || { echo "The key already exists in $dir. It is not replaced. (To send the existing key to GitHub again: bash tools/android-key/make-key.sh --upload)" >&2; exit 1; }
  pass="$(cat "$dir/password.txt")"
  export JARVIS_KS_PASS="$pass"
else
  mkdir -p "$dir"; chmod 700 "$dir"
  pass="$(head -c 32 /dev/urandom | base64 | tr -d '+/=\n')"
  export JARVIS_KS_PASS="$pass"
  keytool -genkeypair -noprompt -storetype PKCS12 -keystore "$dir/jarvis-release.jks" -alias jarvis -keyalg RSA -keysize 4096 -validity 36500 \
    -dname 'CN=Jarvis Remote, O=Jarvis' -storepass:env JARVIS_KS_PASS -keypass:env JARVIS_KS_PASS >/dev/null
  printf '%s' "$pass" > "$dir/password.txt"; chmod 600 "$dir/password.txt" "$dir/jarvis-release.jks"
fi
# the fingerprint of the certificate is public information: shown so that it can be compared with the one CI reports
fp="$(keytool -list -v -keystore "$dir/jarvis-release.jks" -storepass:env JARVIS_KS_PASS -alias jarvis 2>/dev/null | sed -n 's/^[[:space:]]*SHA256:[[:space:]]*//p' | head -1)"
if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1 && [ "$(gh api "repos/$repo" --jq .permissions.admin 2>/dev/null)" = "true" ]; then
  base64 -w0 "$dir/jarvis-release.jks" 2>/dev/null | gh secret set ANDROID_KEYSTORE_BASE64 --repo "$repo" >/dev/null || { base64 "$dir/jarvis-release.jks" | tr -d '\n' | gh secret set ANDROID_KEYSTORE_BASE64 --repo "$repo" >/dev/null; }
  printf '%s' "$pass" | gh secret set ANDROID_KEYSTORE_PASSWORD --repo "$repo" >/dev/null
  printf 'jarvis' | gh secret set ANDROID_KEY_ALIAS --repo "$repo" >/dev/null
  echo "Done. Secrets set in $repo: $(gh secret list --repo "$repo" | awk '{print $1}' | grep '^ANDROID_' | tr '\n' ' ')"
  echo "Certificate SHA-256 (public): $fp"
  echo "KEEP the folder $dir (the key and password.txt) in a safe place. Then tell Claude: the key is created."
else
  base64 -w0 "$dir/jarvis-release.jks" 2>/dev/null > "$dir/ANDROID_KEYSTORE_BASE64.txt" || base64 "$dir/jarvis-release.jks" | tr -d '\n' > "$dir/ANDROID_KEYSTORE_BASE64.txt"
  chmod 600 "$dir/ANDROID_KEYSTORE_BASE64.txt"
  echo "The key is created, but the GitHub CLI is missing, not logged in or has no admin right to $repo, so the secrets were NOT set." >&2
  echo "Run once:  gh auth login --web   then   bash tools/android-key/make-key.sh --upload   or add by hand (GitHub > Settings > Secrets and variables > Actions):" >&2
  echo "  ANDROID_KEYSTORE_BASE64 = the content of $dir/ANDROID_KEYSTORE_BASE64.txt, ANDROID_KEYSTORE_PASSWORD = the content of $dir/password.txt, ANDROID_KEY_ALIAS = jarvis" >&2
  echo "Certificate SHA-256 (public): $fp" >&2
  exit 2
fi
