#!/usr/bin/env bash
# Creates the NEW Android release key on YOUR computer and, if the GitHub CLI "gh" is installed and logged in, stores it as the three protected
# secrets of the repository. Nothing is printed or committed; the key and the password stay in ~/jarvis-android-key (back it up: without the
# key the app cannot be updated). Run:  bash tools/android-key/make-key.sh
set -euo pipefail
repo='XasaNXoN96/Jarvis-1.0'
dir="$HOME/jarvis-android-key"
[ -e "$dir/jarvis-release.jks" ] && { echo "The key already exists in $dir. It is not replaced." >&2; exit 1; }
umask 077; mkdir -p "$dir"
pass="$(head -c 24 /dev/urandom | base64 | tr -d '+/=\n')"
keytool -genkeypair -noprompt -storetype PKCS12 -keystore "$dir/jarvis-release.jks" -alias jarvis -keyalg RSA -keysize 4096 -validity 36500 \
  -dname 'CN=Jarvis Remote, O=Jarvis' -storepass "$pass" -keypass "$pass" >/dev/null
printf '%s' "$pass" > "$dir/password.txt"
if command -v gh >/dev/null 2>&1; then
  base64 -w0 "$dir/jarvis-release.jks" | gh secret set ANDROID_KEYSTORE_BASE64 --repo "$repo"
  printf '%s' "$pass" | gh secret set ANDROID_KEYSTORE_PASSWORD --repo "$repo"
  printf 'jarvis' | gh secret set ANDROID_KEY_ALIAS --repo "$repo"
  echo "Done: the three secrets are set in $repo. KEEP $dir (key + password.txt) in a safe place."
else
  base64 -w0 "$dir/jarvis-release.jks" > "$dir/ANDROID_KEYSTORE_BASE64.txt"
  echo "gh is not installed. Add the secrets by hand: ANDROID_KEYSTORE_BASE64 = $dir/ANDROID_KEYSTORE_BASE64.txt, ANDROID_KEYSTORE_PASSWORD = $dir/password.txt, ANDROID_KEY_ALIAS = jarvis. Then delete ANDROID_KEYSTORE_BASE64.txt."
fi
