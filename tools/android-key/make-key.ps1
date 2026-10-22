# Creates the NEW Android release key on YOUR Windows computer and stores it as the three protected secrets of the repository (GitHub CLI "gh").
# - a new random key and password every run; an existing key is never replaced;
# - the password goes to keytool through an environment variable (never on a command line) and is never printed;
# - the key and the password stay in "$HOME\jarvis-android-key" (only your account can read it): back that folder up, without the key the app cannot be updated;
# - nothing is committed.
# Run (PowerShell):  powershell -ExecutionPolicy Bypass -File tools\android-key\make-key.ps1
param([switch]$Upload)
$ErrorActionPreference = 'Stop'
$repo = 'XasaNXoN96/Jarvis-1.0'
$dir = if ($env:JARVIS_KEY_DIR) { $env:JARVIS_KEY_DIR } else { Join-Path $HOME 'jarvis-android-key' }

# keytool: the PATH, JAVA_HOME, Android Studio, a Temurin/Oracle JDK
function Find-Keytool {
  $c = Get-Command keytool -ErrorAction SilentlyContinue; if ($c) { return $c.Source }
  $cands = @()
  if ($env:JAVA_HOME) { $cands += (Join-Path $env:JAVA_HOME 'bin\keytool.exe') }
  $cands += (Join-Path $env:ProgramFiles 'Android\Android Studio\jbr\bin\keytool.exe')
  foreach ($root in @("$env:ProgramFiles\Eclipse Adoptium", "$env:ProgramFiles\Java", "$env:ProgramFiles\Microsoft")) {
    if (Test-Path $root) { $cands += (Get-ChildItem $root -Directory -ErrorAction SilentlyContinue | ForEach-Object { Join-Path $_.FullName 'bin\keytool.exe' }) }
  }
  foreach ($k in $cands) { if ($k -and (Test-Path $k)) { return $k } }
  return $null
}
$keytool = Find-Keytool
if (-not $keytool) { throw "keytool (a JDK) was not found. Install it:  winget install EclipseAdoptium.Temurin.17.JDK  and run this script again." }
$jks = Join-Path $dir 'jarvis-release.jks'
$passFile = Join-Path $dir 'password.txt'
if (Test-Path $jks) {
  # an existing key is never replaced; with -Upload it is only (re)sent to the secrets (e.g. after "gh auth login")
  if (-not ($Upload -and (Test-Path $passFile))) { throw "The key already exists in $dir. It is not replaced. (To send the existing key to GitHub again: add -Upload)" }
  $pass = [IO.File]::ReadAllText($passFile)
} else {
  New-Item -ItemType Directory -Force $dir | Out-Null
  # only the current user (and the system) may read the folder
  & icacls $dir /inheritance:r /grant:r "$($env:USERNAME):(OI)(CI)F" "SYSTEM:(OI)(CI)F" | Out-Null
  $bytes = New-Object byte[] 32; [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
  $pass = [Convert]::ToBase64String($bytes).Replace('+','A').Replace('/','b').Replace('=','')
}
$env:JARVIS_KS_PASS = $pass
try {
  if (-not (Test-Path $jks)) {
    & $keytool -genkeypair -noprompt -storetype PKCS12 -keystore $jks -alias jarvis -keyalg RSA -keysize 4096 -validity 36500 -dname 'CN=Jarvis Remote, O=Jarvis' '-storepass:env' JARVIS_KS_PASS '-keypass:env' JARVIS_KS_PASS | Out-Null
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path $jks)) { throw 'keytool could not create the key.' }
    [IO.File]::WriteAllText($passFile, $pass)
  }
  # the fingerprint of the certificate is public information: shown to compare it with the one CI reports
  $fp = (& $keytool -list -v -keystore $jks '-storepass:env' JARVIS_KS_PASS -alias jarvis 2>$null | Select-String 'SHA256:' | Select-Object -First 1).ToString().Trim()
} finally { Remove-Item Env:JARVIS_KS_PASS -ErrorAction SilentlyContinue }

$b64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes($jks))
$gh = Get-Command gh -ErrorAction SilentlyContinue
$ready = $false
if ($gh) {
  & gh auth status *> $null
  if ($LASTEXITCODE -ne 0) { Write-Host 'GitHub login is needed once (a browser window opens):'; & gh auth login --web -h github.com; }
  $admin = (& gh api "repos/$repo" --jq .permissions.admin 2>$null)
  $ready = ($LASTEXITCODE -eq 0 -and $admin -eq 'true')
}
if ($ready) {
  # the values go to "gh secret set" from FILES through cmd's "<": a pipe from PowerShell would add a line break to the value
  $b64File = Join-Path $dir 'ANDROID_KEYSTORE_BASE64.txt'
  [IO.File]::WriteAllText($b64File, $b64)
  & cmd /c "gh secret set ANDROID_KEYSTORE_BASE64 --repo $repo < `"$b64File`"" | Out-Null
  if ($LASTEXITCODE -ne 0) { throw 'Could not set ANDROID_KEYSTORE_BASE64.' }
  & cmd /c "gh secret set ANDROID_KEYSTORE_PASSWORD --repo $repo < `"$passFile`"" | Out-Null
  if ($LASTEXITCODE -ne 0) { throw 'Could not set ANDROID_KEYSTORE_PASSWORD.' }
  & gh secret set ANDROID_KEY_ALIAS --repo $repo --body jarvis | Out-Null
  Remove-Item $b64File -ErrorAction SilentlyContinue
  $names = (& gh secret list --repo $repo | ForEach-Object { ($_ -split '\s+')[0] } | Where-Object { $_ -like 'ANDROID_*' }) -join ' '
  Write-Host "Done. Secrets set in ${repo}: $names"
  Write-Host "Certificate $fp (public)"
  Write-Host "KEEP the folder $dir (the key and password.txt) in a safe place. Then tell Claude: the key is created."
} else {
  Set-Content -NoNewline -Path (Join-Path $dir 'ANDROID_KEYSTORE_BASE64.txt') -Value $b64
  Write-Host "The key is created, but the secrets were NOT set (the GitHub CLI is missing, you are not logged in, or you have no admin right to $repo)."
  Write-Host "Option 1:  winget install GitHub.cli ; gh auth login --web ; then run this script again with -Upload"
  Write-Host "Option 2 (by hand): GitHub > Settings > Secrets and variables > Actions > New repository secret: ANDROID_KEYSTORE_BASE64 = the text of ANDROID_KEYSTORE_BASE64.txt, ANDROID_KEYSTORE_PASSWORD = the text of password.txt, ANDROID_KEY_ALIAS = jarvis (files in $dir)."
  Write-Host "Certificate $fp (public)"
  exit 2
}
