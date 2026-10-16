# Jarvis License Server: локальный запуск ДЛЯ ПРОВЕРКИ на этом компьютере (слушает только 127.0.0.1, из сети недоступен).
# Запуск: щёлкните правой кнопкой по файлу -> «Выполнить с помощью PowerShell», либо в PowerShell:  .\local-license-server.ps1
# Это не боевой сервер: HTTPS нет, ключи и база лежат в папке пользователя. Для настоящей работы см. docs\licensing\DEPLOYMENT.md.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$exe = Join-Path $here 'jarvis-license-server.exe'
if (-not (Test-Path $exe)) { throw "Рядом со скриптом нет jarvis-license-server.exe" }

$dir = Join-Path $env:LOCALAPPDATA 'JarvisLicenseServerTest'
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$env:JARVIS_LS_DB = Join-Path $dir 'licenses.sqlite'
$env:JARVIS_LS_SIGNING_KEY_FILE = Join-Path $dir 'signing.key'
$env:JARVIS_LS_PEPPER_FILE = Join-Path $dir 'pepper.key'
$env:JARVIS_LS_BIND = '127.0.0.1:8787'
$env:JARVIS_LS_RENEW_INFO = 'Это проверочный сервер: ключи продления выдаёт владелец в менеджере лицензий.'

# база, тарифы и два файла секретов (создаются один раз; повторный запуск ничего не сбрасывает)
& $exe init
if ($LASTEXITCODE -ne 0) { throw 'Не удалось подготовить сервер (init).' }

# токен администратора: показывается ОДИН раз; сервер хранит только его отпечаток
$marker = Join-Path $dir 'admin-created.txt'
if (-not (Test-Path $marker)) {
    Write-Host ''
    Write-Host '=== ТОКЕН АДМИНИСТРАТОРА (скопируйте сейчас, потом он не показывается) ===' -ForegroundColor Yellow
    & $exe admin-create owner
    if ($LASTEXITCODE -ne 0) { throw 'Не удалось создать токен администратора.' }
    Set-Content -Path $marker -Value 'токен был создан; новый: удалите этот файл и базу licenses.sqlite или используйте admin-create'
    Write-Host '=========================================================================' -ForegroundColor Yellow
}

Write-Host ''
Write-Host 'Сервер: http://127.0.0.1:8787  (окно не закрывайте: пока оно открыто, сервер работает)' -ForegroundColor Green
Write-Host 'В Jarvis Admin: «Лицензии» -> адрес http://127.0.0.1:8787 и токен администратора -> «Сохранить».'
Write-Host "Данные сервера: $dir"
Write-Host ''
& $exe serve
