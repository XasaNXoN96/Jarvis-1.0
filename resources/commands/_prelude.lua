-- Shared helpers, placed in front of every Lua command script (see lua/engine.rs).
local WIN = jarvis.system.platform == "windows"
local RU = jarvis.context.language == "ru" or jarvis.context.language == "ua"
local function T(ru, en) if RU then return ru end return en end
local phrase = (jarvis.context.phrase or ""):lower()

-- run a program (no shell). On non-Windows systems only logs what would be done.
local function run(program, args)
  if not WIN then
    jarvis.log("info", "[not windows, skipped] " .. program .. " " .. table.concat(args or {}, " "))
    return { success = true, code = 0, stdout = "", stderr = "" }
  end
  return jarvis.system.run(program, args or {})
end

-- open a URL / file / program through the shell
local function open(target)
  if not WIN then
    jarvis.log("info", "[not windows, skipped] open " .. target)
    return true
  end
  if not jarvis.system.open(target) then
    -- stop the script: do not announce "opening" when Windows could not open it
    error(T("Не удалось открыть: ", "Could not open: ") .. target)
  end
  return true
end

-- a little Jarvis humour: now and then (about 1 in 5) a dry remark is added to the answer.
-- kinds: timer, reminder, alarm, power, volume_max, volume_mute, game, program, website, note, lock, generic
local QUIPS = {
  timer      = { ru = { "Время пошло, сэр.", "Постараюсь не опаздывать, в отличие от некоторых." },
                 en = { "The clock is ticking, sir.", "I'll try not to be late, unlike some people." } },
  reminder   = { ru = { "Запомню. Забывчивость, к счастью, не мой недостаток.", "Будет исполнено, сэр." },
                 en = { "Noted. Forgetting is, fortunately, not one of my flaws.", "Consider it done, sir." } },
  alarm      = { ru = { "Постараюсь разбудить как можно деликатнее. Нет, не постараюсь.", "Будильник установлен. Вставать всё равно вам." },
                 en = { "I'll wake you as gently as possible. Actually, no.", "Alarm set. Getting up is still your job." } },
  power      = { ru = { "Надеюсь, вы всё сохранили. Хотя я бы не рассчитывал.", "Спокойной ночи, сэр. Не засиживайтесь." },
                 en = { "I hope you saved everything. I wouldn't count on it.", "Good night, sir. Don't stay up too late." } },
  volume_max = { ru = { "Соседи оценят вашу музыкальность.", "Громко. Очень громко. Как скажете." },
                 en = { "The neighbours will appreciate your taste in music.", "Loud. Very loud. As you wish." } },
  volume_mute= { ru = { "Тишина. Наконец-то можно подумать.", "Как скажете. Я и сам люблю тишину." },
                 en = { "Silence. At last, room to think.", "As you wish. I like quiet, too." } },
  game       = { ru = { "Только не забывайте дышать, сэр.", "Постараюсь вовремя вытащить вас из виртуального мира." },
                 en = { "Just remember to breathe, sir.", "I'll do my best to pull you out of the virtual world on time." } },
  program    = { ru = { "Уже открываю. Надеюсь, это для работы.", "Как скажете, сэр." },
                 en = { "Opening it. I hope it's for work.", "As you wish, sir." } },
  website    = { ru = { "Интернет, к счастью, пока на месте.", "Как скажете, сэр." },
                 en = { "The internet is still there, fortunately.", "As you wish, sir." } },
  note       = { ru = { "Записал. Мысль, должен признать, неплохая.", "Записано. Кто-то же должен помнить." },
                 en = { "Written down. A decent thought, I must admit.", "Noted. Someone has to remember." } },
  lock       = { ru = { "Рабочее место под защитой, сэр.", "Заблокировано. Секреты в безопасности." },
                 en = { "Your workstation is secured, sir.", "Locked. Your secrets are safe." } },
  generic    = { ru = { "К вашим услугам, сэр.", "Всегда пожалуйста." },
                 en = { "At your service, sir.", "You're welcome." } },
}
local QUIP_CHANCE = 0.2

local function quip(kind)
  local set = QUIPS[kind]
  if not set then return nil end
  math.randomseed(tonumber(jarvis.context.time.timestamp) or 0)
  math.random(); math.random()
  if math.random() > QUIP_CHANCE then return nil end
  local list = RU and set.ru or set.en
  return list[math.random(#list)]
end

-- tell the user: console + notification + voice.
--   spoken: text to say aloud (default: msg), or false to stay silent
--   kind:   occasionally adds a joke of this kind to the spoken answer
local function say(title, msg, spoken, kind)
  jarvis.out(title .. ": " .. msg)
  jarvis.system.notify(title, msg)
  if spoken == false then return end
  local text = spoken or msg
  local joke = kind and quip(kind) or nil
  if joke then
    jarvis.out("  " .. joke)
    text = text .. ". " .. joke
  end
  jarvis.speak(text)
end

local function words()
  local list = {}
  for w in phrase:gmatch("%S+") do
    list[#list + 1] = (w:gsub("[%.,!%?;:]+$", ""))
  end
  return list
end

-- true when some word of the phrase starts with `stem`
local function has_stem(stem)
  for _, w in ipairs(words()) do
    if w:sub(1, #stem) == stem then return true end
  end
  return false
end

local function plural_ru(n, one, few, many)
  local m100, m10 = n % 100, n % 10
  if m100 >= 11 and m100 <= 19 then return many end
  if m10 == 1 then return one end
  if m10 >= 2 and m10 <= 4 then return few end
  return many
end

-- 5400 -> "1 час 30 минут"
local function fmt_duration(secs)
  local h = secs // 3600
  local m = (secs % 3600) // 60
  local s = secs % 60
  local parts = {}
  if h > 0 then parts[#parts + 1] = h .. " " .. (RU and plural_ru(h, "час", "часа", "часов") or (h == 1 and "hour" or "hours")) end
  if m > 0 then parts[#parts + 1] = m .. " " .. (RU and plural_ru(m, "минуту", "минуты", "минут") or (m == 1 and "minute" or "minutes")) end
  if s > 0 and h == 0 then parts[#parts + 1] = s .. " " .. (RU and plural_ru(s, "секунду", "секунды", "секунд") or (s == 1 and "second" or "seconds")) end
  return table.concat(parts, " ")
end

-- remove stop words anywhere in a phrase, returns the remaining text
local function strip_words(text, stop)
  local set = {}
  for _, w in ipairs(stop) do set[w] = true end
  local out = {}
  for w in text:gmatch("%S+") do
    local bare = (w:gsub("[%.,!%?;:]+$", ""))
    if not set[bare] then out[#out + 1] = w end
  end
  return table.concat(out, " ")
end

-- remove stop words only at the START of a phrase (the rest stays untouched)
local function strip_leading(text, stop)
  local set = {}
  for _, w in ipairs(stop) do set[w] = true end
  local out, started = {}, false
  for w in text:gmatch("%S+") do
    local bare = (w:gsub("[%.,!%?;:]+$", ""))
    if started or not set[bare] then
      started = true
      out[#out + 1] = w
    end
  end
  return table.concat(out, " ")
end

local PS = { "-NoProfile", "-ExecutionPolicy", "Bypass" }
local function powershell(command)
  local args = { table.unpack(PS) }
  args[#args + 1] = "-Command"
  args[#args + 1] = command
  return run("powershell", args)
end
local function ps_file(name, extra)
  local args = { table.unpack(PS) }
  args[#args + 1] = "-File"
  args[#args + 1] = jarvis.context.command_path .. "/" .. name
  for _, a in ipairs(extra or {}) do args[#args + 1] = a end
  return run("powershell", args)
end
