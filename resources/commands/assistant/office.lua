-- Dictation and documents: "открой Word", "пиши" (диктовка до «заканчивай»), "ячейка а1 пиши 5",
-- "ячейка c1 сделай а плюс б", "новый документ"; and the answers to "что сделать с файлом?".
-- One script for: assistant_office_dictate, assistant_office_cell, assistant_office_new and
-- assistant_session (every phrase while a session is open, see session.rs).

local id = jarvis.context.command_id
local TITLE = T("Документы", "Documents")

-- ---------- helpers ----------
local B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
local function b64(data)
  local out = {}
  for i = 1, #data, 3 do
    local a, b, c = data:byte(i, i + 2)
    local n = (a << 16) | ((b or 0) << 8) | (c or 0)
    local c1, c2, c3, c4 = (n >> 18) & 63, (n >> 12) & 63, (n >> 6) & 63, n & 63
    out[#out + 1] = B64:sub(c1 + 1, c1 + 1) .. B64:sub(c2 + 1, c2 + 1) ..
                    (b and B64:sub(c3 + 1, c3 + 1) or "=") .. (c and B64:sub(c4 + 1, c4 + 1) or "=")
  end
  return table.concat(out)
end

-- Word / Excel through COM. Returns ok, output, friendly error
local function office(app, action, a1, a2, a3)
  local args = { "-App", app, "-Action", action }
  if a1 then args[#args + 1] = "-Arg1"; args[#args + 1] = a1 end
  if a2 then args[#args + 1] = "-Arg2"; args[#args + 1] = a2 end
  if a3 then args[#args + 1] = "-Arg3"; args[#args + 1] = a3 end
  local r = ps_file("office.ps1", args)
  if r.success then return true, ((r.stdout or ""):gsub("%s+$", "")), nil end
  local err = r.stderr or ""
  if err:find("NOTINSTALLED") then
    return false, "", app == "word" and T("Microsoft Word не установлен", "Microsoft Word is not installed")
                              or T("Microsoft Excel не установлен", "Microsoft Excel is not installed")
  end
  return false, "", (err:gsub("%s+$", ""):sub(1, 200))
end

-- a line in the window only (no toast notification for every cell / sentence)
local function say_quiet(msg) jarvis.out(msg) end

local function tokens_of(text)
  text = text:lower():gsub("ё", "е"):gsub("[%.,!%?;:]", " ")
  local t = {}
  for w in text:gmatch("%S+") do t[#t + 1] = w end
  return t
end

local function any_word(tokens, set)
  for _, w in ipairs(tokens) do if set[w] then return true end end
  return false
end

local function set_of(list) local s = {} for _, w in ipairs(list) do s[w] = true end return s end

-- ending the dictation. "заканчивай" etc. count at the end of a phrase or in a short one; plain words that can
-- occur in a text ("все", "конец", "стоп") only when they are (almost) the whole phrase
local STRONG_STOP = set_of({ "заканчивай", "закончи", "закончить", "закончили", "заканчиваем", "завершай", "завершить", "завершили" })
local WEAK_STOP = set_of({ "хватит", "отбой", "конец", "стоп", "все", "всё", "готово", "достаточно" })
local function is_stop(tokens)
  local n = #tokens
  if n == 0 then return false end
  if n <= 4 and any_word(tokens, STRONG_STOP) then return true end
  if STRONG_STOP[tokens[n]] then return true end
  if n == 1 and WEAK_STOP[tokens[1]] then return true end
  return false
end

-- first letter of a text in capitals (UTF-8: Cyrillic and Latin)
local function cap_first(s)
  local b1, b2 = s:byte(1, 2)
  if not b1 then return s end
  if b1 == 0xD0 and b2 and b2 >= 0xB0 and b2 <= 0xBF then return string.char(0xD0, b2 - 0x20) .. s:sub(3) end
  if b1 == 0xD1 and b2 and b2 >= 0x80 and b2 <= 0x8F then return string.char(0xD0, b2 + 0x20) .. s:sub(3) end
  if b1 == 0xD1 and b2 == 0x91 then return string.char(0xD0, 0x81) .. s:sub(3) end
  if b1 < 0x80 then return s:sub(1, 1):upper() .. s:sub(2) end
  return s
end

-- digits or Russian number words at tokens[i..] -> number, next index (longest match)
local function num_at(tokens, i)
  local best, best_j
  for j = i, math.min(i + 4, #tokens) do
    local n = jarvis.parse.number(table.concat(tokens, " ", i, j))
    if n ~= nil and (tokens[j]:match("^%d[%d%.,]*$") or jarvis.parse.number(tokens[j]) ~= nil) then
      best, best_j = n, j
    elseif best then
      break
    end
  end
  if best then return best, best_j + 1 end
  return nil, i
end

local function fmt_number(n)
  if n == math.floor(n) then return string.format("%d", n) end
  return (tostring(n):gsub(",", "."))
end

-- ---------- dictation: spoken punctuation, capitals ----------
local PUNCT = {
  { { "вопросительный", "знак" }, "?" }, { { "восклицательный", "знак" }, "!" },
  { { "точка", "с", "запятой" }, ";" }, { { "двоеточие" }, ":" }, { { "запятая" }, "," }, { { "запятую" }, "," },
  { { "точка" }, "." }, { { "точку" }, "." }, { { "тире" }, " —" }, { { "многоточие" }, "…" },
  { { "новая", "строка" }, "\n" }, { { "новую", "строку" }, "\n" }, { { "перенос", "строки" }, "\n" },
  { { "новый", "абзац" }, "\n\n" }, { { "красная", "строка" }, "\n" },
}

-- "привет как дела вопросительный знак" -> "Привет как дела?" ; remembers the last typed character
local function format_dictation(text)
  local toks = {}
  for w in text:lower():gsub("ё", "е"):gmatch("%S+") do toks[#toks + 1] = w end
  local out, i = {}, 1
  while i <= #toks do
    local matched = false
    for _, p in ipairs(PUNCT) do
      local words, ok = p[1], true
      for k = 1, #words do if toks[i + k - 1] ~= words[k] then ok = false; break end end
      if ok then
        out[#out + 1] = { punct = p[2] }
        i = i + #words
        matched = true
        break
      end
    end
    if not matched then out[#out + 1] = { word = toks[i] }; i = i + 1 end
  end

  local last = jarvis.state.get("dict_last") or ""
  local parts = {}
  local cap = (last == "" or last:match("[%.!%?\n]$") ~= nil)
  local need_space = (last ~= "" and not last:match("[%s\n%(]$"))
  for _, item in ipairs(out) do
    if item.word then
      local w = item.word
      if cap then w = cap_first(w); cap = false end
      parts[#parts + 1] = (need_space and " " or "") .. w
      need_space = true
    else
      local p = item.punct
      parts[#parts + 1] = p
      if p == "\n" or p == "\n\n" then need_space = false; cap = true
      elseif p:match("[%.!%?]") then need_space = true; cap = true
      elseif p == " —" then need_space = true
      else need_space = true end
    end
  end
  local result = table.concat(parts)
  if result ~= "" then
    -- the last whole UTF-8 character (a cut multi-byte character cannot be stored)
    jarvis.state.set("dict_last", result:match("[%z\1-\127\194-\244][\128-\191]*$") or "")
  end
  return result
end

-- ---------- where to write ----------
local APP_WORDS = {
  word = { "ворд", "ворде", "word", "документ", "документе", "вордом" },
  notepad = { "блокнот", "блокноте", "notepad", "блокнота" },
  excel = { "эксель", "экселе", "excel", "таблица", "таблице", "таблицу", "эксел", "экселя" },
}
local function target_from(tokens)
  for app, words in pairs(APP_WORDS) do
    if any_word(tokens, set_of(words)) then return app end
  end
  return nil
end

local function type_into(target, text)
  if text == "" then return true end
  if target == "word" then
    local ok, _, err = office("word", "text", b64(text))
    if not ok then say_quiet(T("Не получилось записать в Word: ", "Could not write to Word: ") .. tostring(err)) end
    return ok
  else
    local ok, err = jarvis.system.type_text(text, "Notepad")
    if not ok then
      say_quiet(T("Не получилось записать в Блокнот (он открыт?): ", "Could not write to Notepad (is it open?): ") .. tostring(err))
    end
    return ok
  end
end

-- ---------- Excel: "ячейка а1 пиши 5", "ячейка c1 сделай а плюс б" ----------
local LETTERS = {
  ["а"] = "A", ["a"] = "A", ["бэ"] = "B", ["б"] = "B", ["бе"] = "B", ["би"] = "B", ["b"] = "B",
  ["цэ"] = "C", ["це"] = "C", ["ц"] = "C", ["c"] = "C", ["си"] = "C", ["с"] = "C",
  ["дэ"] = "D", ["де"] = "D", ["д"] = "D", ["d"] = "D", ["ди"] = "D", ["е"] = "E", ["e"] = "E",
  ["эф"] = "F", ["ф"] = "F", ["f"] = "F", ["гэ"] = "G", ["г"] = "G", ["g"] = "G", ["джи"] = "G",
  ["аш"] = "H", ["х"] = "H", ["h"] = "H", ["иск"] = "X", ["x"] = "X", ["y"] = "Y", ["z"] = "Z",
}
local WRITE = set_of({ "пиши", "запиши", "введи", "впиши", "напиши", "поставь", "равно", "равна", "значение", "внеси", "запишите" })
local FORMULA = set_of({ "сделай", "посчитай", "считай", "вычисли", "формула", "формулу", "подсчитай", "сложи", "рассчитай" })
local FUNCS = { ["сумма"] = "SUM", ["суммы"] = "SUM", ["сумму"] = "SUM", ["среднее"] = "AVERAGE", ["максимум"] = "MAX", ["максимальное"] = "MAX",
                ["минимум"] = "MIN", ["минимальное"] = "MIN", ["количество"] = "COUNT" }
local OPS = { ["плюс"] = "+", ["минус"] = "-", ["умножить"] = "*", ["умножь"] = "*", ["умноженное"] = "*", ["разделить"] = "/", ["раздели"] = "/", ["делить"] = "/" }

-- cell reference at tokens[i] -> col, row (or nil), next index
local function parse_ref(tokens, i)
  local t = tokens[i]
  if not t then return nil end
  local letters, digits = t:match("^(%D+)(%d+)$")
  if letters and LETTERS[letters] then return LETTERS[letters], tonumber(digits), i + 1 end
  if LETTERS[t] then
    local n, ni = num_at(tokens, i + 1)
    if n and n == math.floor(n) and n >= 1 and n <= 1000 then return LETTERS[t], math.floor(n), ni end
    return LETTERS[t], nil, i + 1
  end
  return nil
end

-- formula from tokens[i..]: operands (cell / number), + - * /, functions over a range -> "=A1+B1", or nil, error
local function parse_formula(tokens, i, default_row)
  local f, expect_operand = {}, true
  local function ref_text(col, row) return col .. tostring(row or default_row) end
  while i <= #tokens do
    local t = tokens[i]
    if expect_operand then
      if FUNCS[t] then
        i = i + 1
        if tokens[i] == "от" or tokens[i] == "с" then i = i + 1 end
        local c1, r1, ni = parse_ref(tokens, i)
        if not c1 then return nil, T("не понял диапазон", "range not understood") end
        i = ni
        if tokens[i] == "до" or tokens[i] == "по" then i = i + 1 end
        local c2, r2, ni2 = parse_ref(tokens, i)
        if not c2 then return nil, T("не понял конец диапазона", "end of the range not understood") end
        i = ni2
        f[#f + 1] = FUNCS[t] .. "(" .. ref_text(c1, r1) .. ":" .. ref_text(c2, r2) .. ")"
      else
        local c, r, ni = parse_ref(tokens, i)
        if c then
          f[#f + 1] = ref_text(c, r); i = ni
        else
          local n, nj = num_at(tokens, i)
          if n == nil then return nil, T("не понял «" .. t .. "»", "did not understand '" .. t .. "'") end
          f[#f + 1] = fmt_number(n); i = nj
        end
      end
      expect_operand = false
    else
      local op = OPS[t]
      if not op then
        if t == "и" or t == "на" then i = i + 1 else return nil, T("не понял «" .. t .. "»", "did not understand '" .. t .. "'") end
      else
        f[#f + 1] = op; i = i + 1
        if tokens[i] == "на" then i = i + 1 end   -- "умножить на", "разделить на"
        expect_operand = true
      end
    end
  end
  if #f == 0 or expect_operand then return nil, T("формула не закончена", "the formula is not finished") end
  return "=" .. table.concat(f), nil
end

local function excel_command(text)
  local _
  local tokens = tokens_of(text)
  local i = 1
  while tokens[i] and (tokens[i] == "в" or tokens[i]:find("^ячейк")) do i = i + 1 end
  local col, row, ni = parse_ref(tokens, i)
  if not col then return false, T("Не понял ячейку. Скажите, например: ячейка А1 пиши 5", "Cell not understood. Say e.g.: cell A1 write 5") end
  i = ni
  local target_row = row or tonumber(jarvis.state.get("xl_row")) or 1
  local cell = col .. tostring(target_row)

  local kw = tokens[i]
  if kw and (WRITE[kw] or FORMULA[kw]) then i = i + 1 end
  local rest = {}
  for k = i, #tokens do rest[#rest + 1] = tokens[k] end
  if #rest == 0 then return false, T("Что записать в " .. cell .. "?", "What to write into " .. cell .. "?") end

  local has_formula = FORMULA[kw] ~= nil
  for _, w in ipairs(rest) do if OPS[w] or FUNCS[w] then has_formula = true end end

  if has_formula then
    local formula, err = parse_formula(rest, 1, target_row)
    if not formula then return false, err end
    local ok, shown, e = office("excel", "formula", cell, b64(formula))
    if not ok then return false, e end
    jarvis.state.set("xl_row", target_row)
    return true, cell .. " " .. formula .. (shown ~= "" and ("  =  " .. shown) or "")
  end

  -- a value: a number (digits / number words) or text
  local all_numbers = true
  for _, w in ipairs(rest) do
    if not (w:match("^%d[%d%.,]*$") or jarvis.parse.number(w) ~= nil) then all_numbers = false; break end
  end
  local ok, e
  if all_numbers then
    local n = jarvis.parse.number(table.concat(rest, " "))
    if n == nil then all_numbers = false end
    if all_numbers then ok, _, e = office("excel", "value", cell, b64(fmt_number(n)), "n"); jarvis.state.set("xl_last", fmt_number(n)) end
  end
  if not all_numbers then
    local value = cap_first(table.concat(rest, " "))
    ok, _, e = office("excel", "value", cell, b64(value), "t")
  end
  if not ok then return false, e end
  jarvis.state.set("xl_row", target_row)
  return true, cell .. " = " .. table.concat(rest, " ")
end

-- ---------- what to do with a found file ----------
local ORDINALS = { ["первый"] = 1, ["первую"] = 1, ["один"] = 1, ["второй"] = 2, ["вторую"] = 2, ["два"] = 2, ["третий"] = 3, ["третью"] = 3, ["три"] = 3,
                   ["четвертый"] = 4, ["четыре"] = 4, ["пятый"] = 5, ["пять"] = 5 }
local function file_session(text)
  local tokens = tokens_of(text)
  local exact = set_of({ "да", "нет", "не", "ничего" })
  local whole = " " .. table.concat(tokens, " ") .. " "
  local has = function(stem)
    if exact[stem] then return whole:find(" " .. stem .. " ", 1, true) ~= nil end
    return whole:find(stem, 1, true) ~= nil
  end
  local list = jarvis.files.results()
  local data = jarvis.session.data()
  local n = tonumber(data:match("(%d+)")) or 1
  for _, w in ipairs(tokens) do if ORDINALS[w] then n = ORDINALS[w] end end
  local item = list[n]

  -- waiting for "да" / "нет" after "удалить?"
  if data:find("^confirm_delete") then
    local negative = has("нет") or has("не") or has("отмен") or has("стоп")
    if not negative and (has("да") or has("удал") or has("конечно") or has("подтверж")) then
      local p = (item and item.path or ""):gsub("/", "\\")
      local script = "Add-Type -AssemblyName Microsoft.VisualBasic; $p = [System.Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('" .. b64(p) .. "')); " ..
        "if (Test-Path -LiteralPath $p -PathType Container) { [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteDirectory($p, 'OnlyErrorDialogs', 'SendToRecycleBin') } " ..
        "else { [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile($p, 'OnlyErrorDialogs', 'SendToRecycleBin') }"
      local r = powershell(script)
      jarvis.session.finish()
      if r.success then say(T("Файл", "File"), T("Перемещён в корзину: ", "Moved to the recycle bin: ") .. (item and item.name or ""), T("Удалил в корзину", "Moved to the recycle bin"))
      else say(T("Файл", "File"), T("Не удалось удалить: ", "Could not delete: ") .. (r.stderr or ""):sub(1, 150)) end
    else
      jarvis.session.finish()
      say(T("Файл", "File"), T("Не удаляю", "Not deleting"), T("Хорошо, не удаляю", "Okay, not deleting"))
    end
    return
  end

  if has("отмен") or has("не над") or has("ничего") or has("нет") or has("отбой") or has("хватит") or has("заканчив") or has("закончи") or has("спасибо") then
    jarvis.session.finish()
    say(T("Файл", "File"), T("Хорошо", "Okay"), T("Хорошо", "Okay"))
    return
  end
  if not item then
    say(T("Файл", "File"), T("Результата номер " .. n .. " нет (найдено " .. #list .. ")", "No result number " .. n))
    return
  end

  if has("запуск") or has("запуст") or has("открой") or has("открыв") or has("открыть") or has("включ") or has("играй") or has("воспроизв") or has("смотр") or has("слуша") then
    jarvis.session.finish()
    local ok = pcall(open, item.path)
    if ok then say(T("Открываю", "Opening"), item.name, T("Открываю " .. item.name, "Opening " .. item.name))
    else say(T("Файл", "File"), T("Не удалось открыть ", "Could not open ") .. item.name) end
  elseif has("папк") then
    jarvis.session.finish()
    launch("explorer.exe", { "/select," .. item.path:gsub("/", "\\") })
    say(T("Папка", "Folder"), item.dir, T("Показываю папку", "Showing the folder"))
  elseif has("копир") or has("отправ") or has("перешл") or has("поделис") then
    jarvis.session.finish()
    local p = item.path:gsub("/", "\\")
    local r = powershell("Set-Clipboard -LiteralPath ([System.Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('" .. b64(p) .. "')))")
    local sending = has("отправ") or has("перешл") or has("поделис")
    if r.success then
      say(T("Файл", "File"), T("Файл в буфере обмена: вставьте его (Ctrl+V) в папку, письмо или мессенджер", "The file is in the clipboard: paste it (Ctrl+V) into a folder, mail or messenger"),
          sending and T("Файл скопирован. Вставьте его в письмо или мессенджер", "The file is copied. Paste it into mail or a messenger") or T("Скопировал. Вставьте куда нужно", "Copied. Paste it where you need"))
    else
      say(T("Файл", "File"), T("Не удалось скопировать", "Could not copy"))
    end
  elseif has("удал") or has("сотр") or has("убер") then
    jarvis.session.set_data(tostring(n) == "" and "confirm_delete" or ("confirm_delete:" .. n))
    say(T("Файл", "File"), T("Удалить «" .. item.name .. "» в корзину? Скажите да или нет.", "Delete '" .. item.name .. "' to the recycle bin? Say yes or no."),
        T("Удалить " .. item.name .. " в корзину? Скажите да или нет", "Delete " .. item.name .. " to the recycle bin? Say yes or no"))
  elseif #tokens == 1 and ORDINALS[tokens[1]] then
    jarvis.session.set_data(tostring(n))
    say(T("Файл", "File"), T("Выбран " .. n .. ": " .. item.name .. ". Что сделать?", "Selected " .. n .. ": " .. item.name .. ". What to do?"),
        T("Выбрал " .. item.name .. ". Что сделать?", "Selected " .. item.name .. ". What to do?"))
  else
    -- not an answer about the file: leave the question and handle the phrase as an ordinary command
    jarvis.session.finish()
    jarvis.app.run_text(text)
  end
end

-- ---------- entry points ----------
local function finish_session()
  jarvis.session.finish()
  say(TITLE, T("Диктовка завершена", "Dictation finished"), T("Хорошо, закончил", "Okay, finished"))
end

local function start_dictation(target)
  jarvis.state.set("dict_last", "")
  jarvis.state.set("office_app", target)
  if target == "word" or target == "excel" then
    local ok, _, err = office(target, "ensure")
    if not ok then say(TITLE, tostring(err)); return end
  elseif WIN and not jarvis.system.window_exists("Notepad") then
    launch("notepad.exe", {})
    jarvis.sleep(1200)
  end
  jarvis.session.start("dictation", 600)
  jarvis.session.set_data(target)
  if target == "excel" then
    say(TITLE, T("Таблица Excel. Говорите, например: «ячейка А1 пиши 5», «ячейка C1 сделай А плюс Б». Чтобы закончить, скажите «заканчивай».",
                 "Excel. Say e.g. \"cell A1 write 5\". To finish say \"finish\"."),
        T("Слушаю. Говорите ячейку и значение. Чтобы закончить, скажите заканчивай", "Listening. To finish say finish"))
  else
    say(TITLE, T("Диктуйте. Знаки: «точка», «запятая», «новая строка». Чтобы закончить, скажите «заканчивай».",
                 "Dictate. Say \"period\", \"comma\", \"new line\". To finish say \"finish\"."),
        T("Слушаю, диктуйте. Чтобы закончить, скажите заканчивай", "Listening. To finish say finish"))
  end
end

if id == "assistant_office_dictate" then
  local tokens = tokens_of(phrase)
  local target = target_from(tokens) or jarvis.state.get("office_app")
  if target ~= "word" and target ~= "excel" and target ~= "notepad" then
    say(TITLE, T("Куда писать? Скажите: «пиши в Word», «пиши в блокноте» или «пиши в Excel».", "Where to write? Say: write in Word / Notepad / Excel."),
        T("Куда писать? В ворде, в блокноте или в экселе?", "Where to write? Word, Notepad or Excel?"))
    return
  end
  start_dictation(target)

elseif id == "assistant_office_cell" then
  local ok, msg = excel_command(phrase)
  if ok then
    say_quiet(T("Ячейка: ", "Cell: ") .. msg)
    if jarvis.session.kind() ~= "dictation" then
      jarvis.state.set("office_app", "excel")
      jarvis.session.start("dictation", 600)
      jarvis.session.set_data("excel")
      jarvis.out(T("Продолжайте: говорите следующие ячейки. Чтобы закончить, скажите «заканчивай».", "Go on with the next cells. To finish say \"finish\"."))
    end
  else
    say(TITLE, msg)
  end

elseif id == "assistant_office_new" then
  local tokens = tokens_of(phrase)
  local app = (any_word(tokens, set_of({ "таблицу", "таблица", "эксель", "excel", "эксел", "книгу", "книга" })) and "excel") or "word"
  local ok, _, err = office(app, "new")
  if ok then
    jarvis.state.set("office_app", app)
    jarvis.state.set("dict_last", "")
    say(TITLE, app == "excel" and T("Новая таблица создана", "New workbook created") or T("Новый документ создан", "New document created"),
        app == "excel" and T("Новая таблица готова", "New workbook ready") or T("Новый документ готов. Скажите пиши, чтобы диктовать", "New document ready. Say write to dictate"))
  else
    say(TITLE, tostring(err))
  end

else -- assistant_session: every phrase while a session is open
  local kind = jarvis.session.kind()
  if kind == "file" then
    file_session(phrase)
  elseif kind == "dictation" then
    local tokens = tokens_of(phrase)
    local target = jarvis.session.data()
    if is_stop(tokens) then finish_session(); return end
    if any_word(tokens, set_of({ "новый", "новая", "новую", "новое" })) and any_word(tokens, set_of({ "документ", "таблица", "таблицу", "книгу", "лист" })) then
      local ok, _, err = office(target == "excel" and "excel" or "word", "new")
      jarvis.state.set("dict_last", "")
      say_quiet(ok and T("Создан новый документ", "New document created") or tostring(err))
      return
    end
    if target == "excel" then
      local ok, msg = excel_command(phrase)
      if ok then say_quiet(msg) else say_quiet(msg) end
    else
      local text = format_dictation(phrase)
      if type_into(target, text) then jarvis.out("✎ " .. text:gsub("\n", " ↵ ")) end
    end
  else
    -- nothing is open any more
    jarvis.session.finish()
  end
end
