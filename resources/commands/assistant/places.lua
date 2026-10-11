-- open folders, Windows Settings pages and system tools: "открой загрузки", "открой настройки звука",
-- "диспетчер устройств", "открой диск D"

local PLACES = {
  { ru = { "загрузк", "загруженн" }, en = { "downloads" }, kind = "folder", target = "shell:Downloads", name_ru = "Загрузки", name_en = "Downloads" },
  { ru = { "документ" }, en = { "documents" }, kind = "folder", target = "shell:Personal", name_ru = "Документы", name_en = "Documents" },
  { ru = { "рабоч стол" }, en = { "desktop" }, kind = "folder", target = "shell:Desktop", name_ru = "Рабочий стол", name_en = "Desktop" },
  { ru = { "картинк", "изображен", "фотограф", "фотк" }, en = { "pictures", "photos" }, kind = "folder", target = "shell:My Pictures", name_ru = "Изображения", name_en = "Pictures" },
  { ru = { "музык" }, en = { "music folder" }, kind = "folder", target = "shell:My Music", name_ru = "Музыка", name_en = "Music" },
  { ru = { "видео" }, en = { "videos folder" }, kind = "folder", target = "shell:My Video", name_ru = "Видео", name_en = "Videos" },
  { ru = { "корзин" }, en = { "recycle bin" }, kind = "folder", target = "shell:RecycleBinFolder", name_ru = "Корзина", name_en = "Recycle Bin" },
  { ru = { "этот компьютер", "мой компьютер" }, en = { "this pc", "my computer" }, kind = "folder", target = "shell:MyComputerFolder", name_ru = "Этот компьютер", name_en = "This PC" },
  { ru = { "проводник" }, en = { "file explorer", "explorer" }, kind = "folder", target = "shell:Personal", name_ru = "Проводник", name_en = "Explorer" },
  { ru = { "звук настройк", "настройк звук" }, en = { "sound settings" }, kind = "open", target = "ms-settings:sound", name_ru = "Настройки звука", name_en = "Sound settings" },
  { ru = { "настройк экран", "настройк дисплей", "разрешен экран" }, en = { "display settings", "screen settings" }, kind = "open", target = "ms-settings:display", name_ru = "Настройки экрана", name_en = "Display settings" },
  { ru = { "настройк сет", "настройк интернет", "настройк wifi", "настройк вай" }, en = { "network settings", "wifi settings", "internet settings" }, kind = "open", target = "ms-settings:network", name_ru = "Настройки сети", name_en = "Network settings" },
  { ru = { "bluetooth настройк", "настройк bluetooth", "настройк блютуз", "блютуз настройк" }, en = { "bluetooth settings" }, kind = "open", target = "ms-settings:bluetooth", name_ru = "Bluetooth", name_en = "Bluetooth settings" },
  { ru = { "обновлен windows", "обновлен виндовс", "центр обновлен", "настройк обновлен" }, en = { "windows update", "update settings" }, kind = "open", target = "ms-settings:windowsupdate", name_ru = "Обновление Windows", name_en = "Windows Update" },
  { ru = { "настройк приложен", "установленн программ", "удалит программ", "удален программ" }, en = { "installed apps", "apps settings" }, kind = "open", target = "ms-settings:appsfeatures", name_ru = "Приложения", name_en = "Apps" },
  { ru = { "настройк батаре", "экономи заряд", "настройк питан", "режим питан", "электропитан" }, en = { "power settings", "battery settings" }, kind = "open", target = "ms-settings:powersleep", name_ru = "Питание", name_en = "Power settings" },
  { ru = { "персонализац", "настройк тем", "смен обои", "смен фон", "настройк фон" }, en = { "personalization", "change wallpaper" }, kind = "open", target = "ms-settings:personalization", name_ru = "Персонализация", name_en = "Personalization" },
  { ru = { "настройк уведомлен" }, en = { "notification settings" }, kind = "open", target = "ms-settings:notifications", name_ru = "Уведомления", name_en = "Notifications" },
  { ru = { "хранилищ", "настройк памят", "освобод мест" }, en = { "storage settings", "free up space" }, kind = "open", target = "ms-settings:storagesense", name_ru = "Хранилище", name_en = "Storage" },
  { ru = { "о системе", "о компьютере", "характеристик компьютер", "характеристик система" }, en = { "about this pc", "system info", "pc specs" }, kind = "open", target = "ms-settings:about", name_ru = "О системе", name_en = "About" },
  { ru = { "настройк язык", "язык и регион", "настройк регион" }, en = { "language settings", "region settings" }, kind = "open", target = "ms-settings:regionlanguage", name_ru = "Язык и регион", name_en = "Language" },
  { ru = { "настройк клавиатур" }, en = { "keyboard settings" }, kind = "open", target = "ms-settings:typing", name_ru = "Клавиатура", name_en = "Keyboard" },
  { ru = { "настройк мыш" }, en = { "mouse settings" }, kind = "open", target = "ms-settings:mousetouchpad", name_ru = "Мышь", name_en = "Mouse" },
  { ru = { "принтер" }, en = { "printers" }, kind = "open", target = "ms-settings:printers", name_ru = "Принтеры", name_en = "Printers" },
  { ru = { "конфиденциальн" }, en = { "privacy settings" }, kind = "open", target = "ms-settings:privacy", name_ru = "Конфиденциальность", name_en = "Privacy" },
  { ru = { "настройк микрофон", "доступ микрофон", "разрешен микрофон" }, en = { "microphone privacy", "microphone permissions" }, kind = "open", target = "ms-settings:privacy-microphone", name_ru = "Доступ к микрофону", name_en = "Microphone access" },
  { ru = { "настройк камер", "доступ камер" }, en = { "camera privacy", "camera permissions" }, kind = "open", target = "ms-settings:privacy-webcam", name_ru = "Доступ к камере", name_en = "Camera access" },
  { ru = { "настройк врем", "дата и врем", "настройк дат" }, en = { "date and time settings", "time settings" }, kind = "open", target = "ms-settings:dateandtime", name_ru = "Дата и время", name_en = "Date and time" },
  { ru = { "ночн свет", "ночн режим" }, en = { "night light" }, kind = "open", target = "ms-settings:nightlight", name_ru = "Ночной свет", name_en = "Night light" },
  { ru = { "автозагрузк", "автозапуск программ" }, en = { "startup apps" }, kind = "open", target = "ms-settings:startupapps", name_ru = "Автозагрузка", name_en = "Startup apps" },
  { ru = { "игров режим windows", "настройк игр" }, en = { "gaming settings" }, kind = "open", target = "ms-settings:gaming-gamemode", name_ru = "Игровой режим", name_en = "Game mode" },
  { ru = { "диспетчер устройств" }, en = { "device manager" }, kind = "open", target = "devmgmt.msc", name_ru = "Диспетчер устройств", name_en = "Device Manager" },
  { ru = { "управлен диск" }, en = { "disk management" }, kind = "open", target = "diskmgmt.msc", name_ru = "Управление дисками", name_en = "Disk Management" },
  { ru = { "служб windows", "список служб", "службы" }, en = { "services" }, kind = "open", target = "services.msc", name_ru = "Службы", name_en = "Services" },
  { ru = { "сетев подключен" }, en = { "network connections" }, kind = "open", target = "ncpa.cpl", name_ru = "Сетевые подключения", name_en = "Network connections" },
  { ru = { "свойств систем" }, en = { "system properties" }, kind = "open", target = "sysdm.cpl", name_ru = "Свойства системы", name_en = "System properties" },
  { ru = { "очистк диск" }, en = { "disk cleanup" }, kind = "open", target = "cleanmgr.exe", name_ru = "Очистка диска", name_en = "Disk Cleanup" },
  { ru = { "монитор ресурс" }, en = { "resource monitor" }, kind = "open", target = "resmon.exe", name_ru = "Монитор ресурсов", name_en = "Resource Monitor" },
  { ru = { "экранн клавиатур" }, en = { "on-screen keyboard" }, kind = "open", target = "osk.exe", name_ru = "Экранная клавиатура", name_en = "On-screen keyboard" },
  { ru = { "лупу", "лупа" }, en = { "magnifier" }, kind = "open", target = "magnify.exe", name_ru = "Лупа", name_en = "Magnifier" },
  { ru = { "брандмауэр", "файрвол" }, en = { "firewall" }, kind = "open", target = "firewall.cpl", name_ru = "Брандмауэр", name_en = "Firewall" },
  { ru = { "магазин" }, en = { "microsoft store", "store" }, kind = "open", target = "ms-windows-store:", name_ru = "Магазин", name_en = "Store" },
  { ru = { "будильник" }, en = { "alarms app" }, kind = "open", target = "ms-clock:", name_ru = "Будильники", name_en = "Alarms & Clock" },
  { ru = { "календар" }, en = { "calendar" }, kind = "open", target = "outlookcal:", name_ru = "Календарь", name_en = "Calendar" },
  { ru = { "камеру", "камера" }, en = { "camera app" }, kind = "open", target = "microsoft.windows.camera:", name_ru = "Камера", name_en = "Camera" },
  { ru = { "карты" }, en = { "maps app" }, kind = "open", target = "bingmaps:", name_ru = "Карты", name_en = "Maps" },
}

-- "открой диск D"
local letter = phrase:match("диск%s+([a-zа-я])%f[%A]") or phrase:match("disk%s+([a-z])%f[%A]") or phrase:match("drive%s+([a-z])%f[%A]")
local CYR = { ["д"] = "D", ["с"] = "C", ["е"] = "E", ["э"] = "E", ["ф"] = "F", ["г"] = "G" }
if letter and (jarvis.context.command_id == "assistant_disk_open") then
  letter = (CYR[letter] or letter):upper()
  if WIN and not jarvis.system.exists(letter .. ":\\") then
    say(T("Диск", "Drive"), T("Диска " .. letter .. ": нет на этом компьютере", "There is no drive " .. letter .. ": on this computer"))
    return
  end
  launch("explorer.exe", { letter .. ":\\" })
  say(T("Диск", "Drive"), letter .. ":", T("Открываю диск " .. letter, "Opening drive " .. letter))
  return
end

local function matches(stems)
  for _, stem in ipairs(stems) do
    local ok = true
    for part in stem:gmatch("%S+") do
      if not has_stem(part) then ok = false; break end
    end
    if ok then return true end
  end
  return false
end

local best, best_len
for _, p in ipairs(PLACES) do
  local list = RU and p.ru or p.en
  for _, stem in ipairs(list) do
    local ok = true
    local len = 0
    for part in stem:gmatch("%S+") do
      len = len + 1
      if not has_stem(part) then ok = false; break end
    end
    if ok and (not best_len or len > best_len) then best, best_len = p, len end
  end
end

if not best then
  say(T("Открыть", "Open"), T("Не знаю, что открыть", "I don't know what to open"))
  return
end

local name = RU and best.name_ru or best.name_en
if best.kind == "folder" then
  launch("explorer.exe", { best.target })
else
  open(best.target)
end
say(T("Открываю", "Opening"), name, T("Открываю " .. name, "Opening " .. name))
