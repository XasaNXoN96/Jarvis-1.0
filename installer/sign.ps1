# Signs the given files with the code-signing certificate of the owner (Authenticode, SHA-256, with a time stamp).
# The certificate comes ONLY from two secrets of the repository: WINDOWS_CERT_PFX_BASE64 (the .pfx file in base64) and
# WINDOWS_CERT_PASSWORD. Without them nothing is signed and the script says so (exit 0): the builds of a fork or of a
# repository without a certificate keep working, their files are simply unsigned (Windows SmartScreen will warn about them).
# With a certificate, every file must be signed and verified, otherwise the script fails (a half signed release is worse than none).
param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Files)
$ErrorActionPreference = "Stop"
$pfx64 = $env:WINDOWS_CERT_PFX_BASE64
$pass = $env:WINDOWS_CERT_PASSWORD
if (-not $pfx64 -or -not $pass) {
  Write-Output "::warning title=code signing::WINDOWS_CERT_PFX_BASE64 / WINDOWS_CERT_PASSWORD are not set: the files are NOT signed ($($Files -join ', '))"
  exit 0
}
$signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" -ErrorAction SilentlyContinue | Sort-Object FullName -Descending | Select-Object -First 1
if (-not $signtool) { Write-Output "::error title=code signing::signtool.exe not found"; exit 1 }
$pfx = Join-Path $env:RUNNER_TEMP "codesign.pfx"
[System.IO.File]::WriteAllBytes($pfx, [System.Convert]::FromBase64String($pfx64))
try {
  foreach ($f in $Files) {
    if (-not (Test-Path $f)) { Write-Output "::error title=code signing::$f does not exist"; exit 1 }
    $signed = $false
    # the time stamp servers are tried one after another (one of them may be down)
    foreach ($ts in "http://timestamp.digicert.com", "http://timestamp.sectigo.com", "http://time.certum.pl") {
      & $signtool.FullName sign /f $pfx /p $pass /fd SHA256 /tr $ts /td SHA256 /d "Jarvis" $f
      if ($LASTEXITCODE -eq 0) { $signed = $true; break }
    }
    if (-not $signed) { Write-Output "::error title=code signing::could not sign $f"; exit 1 }
    & $signtool.FullName verify /pa $f
    if ($LASTEXITCODE -ne 0) { Write-Output "::error title=code signing::the signature of $f does not verify"; exit 1 }
    Write-Output "signed and verified: $f"
  }
} finally {
  Remove-Item $pfx -Force -ErrorAction SilentlyContinue
}
