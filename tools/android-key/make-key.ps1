# Creates the NEW Android release key on YOUR computer and (if the GitHub CLI "gh" is installed and you are logged in) stores it as the three
# protected secrets of the repository. Nothing is printed and nothing is committed: the key and the password stay in the folder
# "$HOME\jarvis-android-key" - copy that folder to a password manager / an offline disk and keep it: without the key the app cannot be updated.
# Run:  powershell -ExecutionPolicy Bypass -File tools\android-key\make-key.ps1
$ErrorActionPreference = 'Stop'
$repo = 'XasaNXoN96/Jarvis-1.0'
$dir = Join-Path $HOME 'jarvis-android-key'
if (Test-Path (Join-Path $dir 'jarvis-release.jks')) { throw "The key already exists in $dir. It is not replaced." }
New-Item -ItemType Directory -Force $dir | Out-Null
$bytes = New-Object byte[] 24; [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
$pass = [Convert]::ToBase64String($bytes).Replace('+','A').Replace('/','b').Replace('=','')
$jks = Join-Path $dir 'jarvis-release.jks'
& keytool -genkeypair -noprompt -storetype PKCS12 -keystore $jks -alias jarvis -keyalg RSA -keysize 4096 -validity 36500 -dname 'CN=Jarvis Remote, O=Jarvis' -storepass $pass -keypass $pass | Out-Null
$b64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes($jks))
Set-Content -NoNewline -Path (Join-Path $dir 'password.txt') -Value $pass
if (Get-Command gh -ErrorAction SilentlyContinue) {
  $b64 | gh secret set ANDROID_KEYSTORE_BASE64 --repo $repo
  $pass | gh secret set ANDROID_KEYSTORE_PASSWORD --repo $repo
  'jarvis' | gh secret set ANDROID_KEY_ALIAS --repo $repo
  Write-Host "Done: the three secrets are set in $repo. KEEP the folder $dir (key + password.txt) in a safe place."
} else {
  Set-Content -NoNewline -Path (Join-Path $dir 'ANDROID_KEYSTORE_BASE64.txt') -Value $b64
  Write-Host "gh is not installed. Add the secrets by hand (GitHub > Settings > Secrets and variables > Actions): ANDROID_KEYSTORE_BASE64 = file ANDROID_KEYSTORE_BASE64.txt, ANDROID_KEYSTORE_PASSWORD = file password.txt, ANDROID_KEY_ALIAS = jarvis. Then delete ANDROID_KEYSTORE_BASE64.txt."
}
