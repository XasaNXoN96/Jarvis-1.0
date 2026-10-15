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
-- BOUND: the name of the document the work mode ("Работа с объектом") is bound to. While it is set, the script
-- only uses the program and the document that are open: a closed program is not started again and the text never
-- lands in another document.
local BOUND = nil

local function office(app, action, a1, a2, a3)
  local args = { "-App", app, "-Action", action }
  if a1 then args[#args + 1] = "-Arg1"; args[#args + 1] = a1 end
  if a2 then args[#args + 1] = "-Arg2"; args[#args + 1] = a2 end
  if a3 then args[#args + 1] = "-Arg3"; args[#args + 1] = a3 end
  if BOUND ~= nil and (app == "word" or app == "excel") then
    args[#args + 1] = "-Strict"
    args[#args + 1] = "-Expect"; args[#args + 1] = b64(BOUND)
  end
  local r = ps_file("office.ps1", args)
  if r.success then return true, ((r.stdout or ""):gsub("%s+$", "")), nil end
  local err = r.stderr or ""
  local name = app == "word" and "Word" or (app == "excel" and "Excel" or T("Блокнот", "Notepad"))
  if err:find("NOTINSTALLED") then
    return false, "", app == "word" and T("Microsoft Word не установлен", "Microsoft Word is not installed")
                              or T("Microsoft Excel не установлен", "Microsoft Excel is not installed")
  elseif err:find("NOAPP") then
    return false, "", name .. T(" закрыт или недоступен. Текст не записан", " is closed or unavailable. Nothing was written"), "NOAPP"
  elseif err:find("NODOC") then
    return false, "", T("В " .. name .. " нет открытого документа. Текст не записан", "No open document in " .. name .. ". Nothing was written"), "NODOC"
  elseif err:find("DOCCHANGED") then
    local other = err:match("DOCCHANGED:([^\r\n]*)") or ""
    return false, "", T("Сейчас активен другой документ «" .. other .. "». Текст не записан: вернитесь к своему документу", "Another document is active now. Nothing was written"), "DOCCHANGED"
  elseif err:find("TEXTCHANGED") then
    return false, "", T("Текст документа изменился, пока я работал. Ничего не заменено.", "The document text changed meanwhile. Nothing replaced."), "TEXTCHANGED"
  elseif err:find("NEEDNAME") then
    return false, "", "NEEDNAME", "NEEDNAME"
  elseif err:find("EXISTS") then
    return false, "", "EXISTS", "EXISTS"
  elseif err:find("NOTSAVED") then
    return false, "", T("Не удалось подтвердить сохранение: проверьте окно сохранения", "Could not confirm the save: check the save window"), "NOTSAVED"
  end
  return false, "", jarvis.text.preview(err, 200)
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
-- "второй", "седьмую", "номер 3", "пять": the number of a result (whole words: "пятница" is not five)
local FILE_ORD = { { "перв", 1 }, { "втор", 2 }, { "трет", 3 }, { "четв", 4 }, { "пят", 5 }, { "шест", 6 }, { "седьм", 7 }, { "восьм", 8 }, { "девят", 9 }, { "десят", 10 } }
local FILE_NUM = { ["один"] = 1, ["одну"] = 1, ["два"] = 2, ["две"] = 2, ["три"] = 3, ["четыре"] = 4, ["пять"] = 5, ["шесть"] = 6, ["семь"] = 7,
                   ["восемь"] = 8, ["девять"] = 9, ["десять"] = 10, ["first"] = 1, ["second"] = 2, ["third"] = 3, ["fourth"] = 4, ["fifth"] = 5 }
local function result_number(w)
  if FILE_NUM[w] then return FILE_NUM[w] end
  local d = tonumber(w)
  if d and d >= 1 and d == math.floor(d) then return d end
  -- an ordinal ending ("второй", "седьмую", "четвёртый"), not another word ("вторник", "пятница")
  local ENDINGS = { ["ой"] = true, ["ая"] = true, ["ую"] = true, ["ое"] = true, ["ый"] = true, ["ий"] = true, ["ья"] = true, ["ью"] = true,
                    ["ье"] = true, ["ого"] = true, ["ому"] = true, ["ертый"] = true, ["ертую"] = true, ["ертая"] = true, ["ертое"] = true, ["ётый"] = true, ["ётую"] = true }
  for _, o in ipairs(FILE_ORD) do
    if w:sub(1, #o[1]) == o[1] and ENDINGS[w:sub(#o[1] + 1)] then return o[2] end
  end
  return nil
end

local function file_session(text)
  local tokens = tokens_of(text)
  local exact = set_of({ "да", "нет", "не", "ничего" })
  local whole = " " .. table.concat(tokens, " ") .. " "
  local has = function(stem)
    if exact[stem] then return whole:find(" " .. stem .. " ", 1, true) ~= nil end
    return whole:find(stem, 1, true) ~= nil
  end
  -- a word of the phrase begins with the stem (not a part inside another word: "сообщение" has no "ещ")
  local has_tok = function(...)
    for _, stem in ipairs({ ... }) do
      for _, w in ipairs(tokens) do if w:sub(1, #stem) == stem then return true end end
    end
    return false
  end
  local short = #tokens <= 4 -- an answer to the question is short; a long phrase is another command
  local list = jarvis.files.results()
  local data = jarvis.session.data()
  -- the chosen result: said now ("второй"), chosen before in this search, or the only one; never a random one
  local n = jarvis.files.selected()
  if data:find("^confirm_delete") then n = tonumber(data:match("(%d+)")) or n end
  local said = false
  if short then
    for _, w in ipairs(tokens) do
      local k = result_number(w)
      if k then n = k; said = true end
    end
  end
  if not n and #list == 1 then n = 1 end
  local item = n and list[n] or nil

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
      else say(T("Файл", "File"), T("Не удалось удалить: ", "Could not delete: ") .. jarvis.text.preview(r.stderr or "", 150), T("Не удалось удалить файл", "Could not delete the file")) end
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
  -- "ещё": the next page; "следующий": the next result (not "ещё громче", not "следующий трек")
  local other = has_tok("громк", "громч", "тише", "звук", "трек", "песн", "композиц", "видео", "вкладк", "окн", "слайд", "страниц")
  if #tokens <= 3 and not other and has_tok("ещ", "больше", "остальн", "дальше") then
    say(T("Файл", "File"), jarvis.files.more(), false)
    return
  end
  if #tokens <= 3 and not other and has_tok("следующ") then
    local m = (n or 0) + 1
    if not list[m] then say(T("Файл", "File"), T("Это был последний результат", "That was the last result")); return end
    jarvis.session.set_data(tostring(m))
    jarvis.files.select(m)
    say(T("Файл", "File"), m .. ". " .. list[m].name .. " — " .. list[m].dir .. T(". Открыть или показать папку?", ". Open it or show the folder?"), T("Номер " .. m .. ". Открыть?", "Number " .. m .. ". Open it?"))
    return
  end
  local wants_action = short and has_tok("запуск", "запуст", "открой", "открыв", "открыть", "папк", "копир", "удал", "сотр", "покаж", "где")
  if not n and wants_action then
    say(T("Файл", "File"), T("Найдено " .. #list .. ". Скажите номер, например «второй».", "Found " .. #list .. ". Say the number."), T("Какой номер?", "Which number?"))
    return
  end
  if n and not item then
    say(T("Файл", "File"), T("Результата номер " .. n .. " нет (найдено " .. #list .. ")", "No result number " .. n))
    return
  end
  if not item then
    -- not an answer about the files: leave the question, the phrase is an ordinary command
    jarvis.session.finish()
    jarvis.app.run_text(text)
    return
  end

  if not short or has_tok("телефон") then
    -- a whole sentence or a phone command ("отправь на телефон …") is not an answer about the file
    jarvis.session.finish()
    jarvis.app.run_text(text)
    return
  end
  -- an answer about this file ("открой второй", "второй"): it is the chosen one from now on; a number inside another
  -- phrase ("таймер на 2 минуты") changes nothing
  if n and item and (wants_action or (said and #tokens <= 2)) then jarvis.files.select(n) end
  if has("запуск") or has("запуст") or has("открой") or has("открыв") or has("открыть") or has("включ") or has("играй") or has("воспроизв") or has("смотр") or has("слуша") then
    jarvis.session.finish()
    local ok = pcall(open, item.path)
    if ok then say(T("Открываю", "Opening"), item.name, T("Запускаю", "Opening"))
    else say(T("Файл", "File"), T("Не удалось открыть ", "Could not open ") .. item.name, T("Не удалось открыть файл", "Could not open the file")) end
  elseif has("папк") or has("где") or has("располож") then
    jarvis.session.finish()
    local shown, why = jarvis.files.reveal(item.path)
    if not shown then say(T("Папка", "Folder"), T("Не удалось показать: ", "Cannot show: ") .. tostring(why)); return end
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
        T("Удалить найденный файл в корзину? Скажите да или нет", "Delete the found file to the recycle bin? Say yes or no"))
  elseif said and #tokens <= 2 then
    jarvis.session.set_data(tostring(n))
    say(T("Файл", "File"), T("Выбран " .. n .. ": " .. item.name .. ". Что сделать?", "Selected " .. n .. ": " .. item.name .. ". What to do?"),
        T("Выбрал номер " .. n .. ". Что сделать?", "Selected number " .. n .. ". What to do?"))
  else
    -- not an answer about the file: leave the question and handle the phrase as an ordinary command
    jarvis.session.finish()
    jarvis.app.run_text(text)
  end
end

-- ---------- "Работа с объектом": a document of Word / Excel / Notepad, dictation until the user finishes ----------
-- Steps (kept in the session data): app_ask -> doc_ask -> (doc_name) -> typing -> ask_end -> (save_name | overwrite | confirm_delete)
-- Nothing is saved, closed or deleted without the user's word; text never goes anywhere but the bound document.
local OBJ_TITLE = T("Работа с объектом", "Object mode")
local OBJ_NAMES = { word = "Word", excel = "Excel", notepad = T("Блокнот", "Notepad") }
local OBJ_EXT = { word = "docx", excel = "xlsx", notepad = "txt" }
local OBJ_FIND_EXT = { word = "docx,doc,rtf", excel = "xlsx,xls,xlsm,csv", notepad = "txt,md,log" }

local function obj_say(text, speech)
  jarvis.object.note("jarvis", text)
  say(OBJ_TITLE, text, speech or text)
end

local function stems_in(tokens, stems)
  for _, w in ipairs(tokens) do
    for _, s in ipairs(stems) do
      if w:sub(1, #s) == s then return true end
    end
  end
  return false
end

local function exact_in(tokens, words)
  local set = set_of(words)
  for _, w in ipairs(tokens) do if set[w] then return true end end
  return false
end

-- "начинаем работу с объектом" / "заканчиваем работу с объектом"
local function is_object_start(tokens)
  return stems_in(tokens, { "объект" }) and stems_in(tokens, { "начин", "начн", "начат", "работ", "режим", "включ", "запуст" })
         and not stems_in(tokens, { "заканч", "законч", "заверш", "прекрат", "выйд", "выход" })
end
local function is_object_end(tokens)
  local n = #tokens
  if n == 0 then return false end
  if stems_in(tokens, { "объект" }) and stems_in(tokens, { "заканч", "законч", "заверш", "прекрат", "выйд", "выход", "конец", "хватит", "стоп" }) then return true end
  if n <= 4 and stems_in(tokens, { "заканч", "законч", "заверш" }) and (stems_in(tokens, { "работ" }) or n <= 2) then return true end
  if n == 1 and WEAK_STOP[tokens[1]] and tokens[1] ~= "все" and tokens[1] ~= "всё" then return true end
  return false
end

-- the answer to a question of the work mode
local function answer_of(tokens)
  local a = {}
  if stems_in(tokens, { "сохран" }) then a.save = true end
  if stems_in(tokens, { "удал", "сотр", "выброс", "уничтож" }) then a.delete = true end
  if stems_in(tokens, { "продолж", "дальше", "редактир", "вернис", "вернём", "вернем" }) then a.continue = true end
  if stems_in(tokens, { "отмен", "отбой", "назад", "передума" }) then a.cancel = true end
  if exact_in(tokens, { "да", "ага", "угу", "конечно", "верно", "согласен", "согласна", "окей", "ок", "подтверждаю" }) or stems_in(tokens, { "подтвер", "уверен" }) then a.yes = true end
  if exact_in(tokens, { "нет", "не", "неа", "ни" }) or stems_in(tokens, { "не над", "не хоч" }) then a.no = true end
  if stems_in(tokens, { "нов", "созда", "пуст", "чист" }) then a.new = true end
  if stems_in(tokens, { "существ", "откр", "имеющ", "уже", "стар", "сохранённ", "сохраненн" }) then a.existing = true end
  return a
end

local function obj_app() return jarvis.state.get("obj_app") end
-- a step of the dialog: dictation may go on for a long time, a question is not left open for long
-- the state machine of the work (objects.rs) follows the steps of the dialog: dictation only in the "typing" step, a
-- question only in the steps that wait for "да" / "нет"
local function eng_sync(s)
  local question = (s == "overwrite" or s == "confirm_delete" or s == "confirm_replace")
  local st = jarvis.object.state()
  if question then
    if st == "DICTATION" or st == "OBJECT_ACTIVE" then jarvis.object.ask(s, s) end
    return
  end
  if st == "AWAITING_CONFIRMATION" then jarvis.object.answer(false); st = jarvis.object.state() end
  if s == "typing" then
    if st == "OBJECT_ACTIVE" then jarvis.object.dictation(true) end
  elseif st == "DICTATION" then
    jarvis.object.dictation(false)
  end
end

local function obj_step(s)
  eng_sync(s)
  jarvis.session.start("object", s == "typing" and 900 or 180)
  jarvis.session.set_data(s)
  jarvis.state.set("obj_unclear", "0")
end

local function obj_stop(msg_text, msg_speech)
  jarvis.object.finish()
  jarvis.session.finish()
  jarvis.state.set("obj_app", "")
  jarvis.state.set("obj_name", "")
  if msg_text then say(OBJ_TITLE, msg_text, msg_speech or msg_text) end
end

-- an answer that is not understood: ask again; after three misses the mode ends (nothing is saved, closed or deleted)
local function obj_unclear(text, speech)
  local n = (tonumber(jarvis.state.get("obj_unclear")) or 0) + 1
  jarvis.state.set("obj_unclear", tostring(n))
  jarvis.session.touch()
  if n >= 3 then
    return obj_stop(T("Не получил ответа, поэтому режим работы с объектом завершён. Документ остался открытым и не тронут.", "No answer: object mode finished. The document is untouched."),
                    T("Не получил ответа. Режим работы с объектом завершён, документ остался открытым", "No answer, object mode finished"))
  end
  obj_say(text, speech)
end


-- info about the open document: ok, { name=, path=, has_path=, saved=, open= }
local function obj_info(app)
  local ok, out, err = office(app, "info")
  if not ok then return false, nil, err end
  local f = {}
  for part in (out .. "|"):gmatch("([^|]*)|") do f[#f + 1] = part end
  if f[1] ~= "1" then return true, { open = false } end
  return true, { open = true, name = f[2] or "", path = f[3] or "", saved = f[4] == "1", has_path = f[5] == "1" }
end

local function obj_enter_typing(app, name, path)
  -- a new object: the context of the previous one is replaced (the engine refuses while a question or a generation is open)
  jarvis.object.reset()
  jarvis.object.open_app(app)
  jarvis.object.bind(app, name or "", path, (app == "word" or app == "excel") and "office-com" or "process")
  jarvis.state.set("obj_app", app)
  jarvis.state.set("obj_name", app == "notepad" and "" or (name or ""))
  jarvis.state.set("dict_last", "")
  jarvis.state.set("office_app", app)
  jarvis.state.set("obj_stamped", "")
  jarvis.state.set("obj_close_after", "")
  jarvis.session.start("object", 900)
  obj_step("typing")
  local what = app == "notepad" and T("Блокнот", "Notepad") or ("«" .. (name or "") .. "»")
  if app == "excel" then
    obj_say(T("Работаю с таблицей " .. what .. ". Диктуйте значения: они пойдут в активную ячейку и вниз; или «ячейка А1 пиши 5». Чтобы закончить, скажите «заканчиваем работу с объектом».",
              "Working with " .. what),
            T("Работаю с таблицей. Диктуйте. Чтобы закончить, скажите: заканчиваем работу с объектом", "Listening"))
  else
    obj_say(T("Работаю с документом " .. what .. ". Диктуйте текст. Знаки: «точка», «запятая», «новая строка»; формат: «включи жирный», «по центру». Чтобы закончить, скажите «заканчиваем работу с объектом».",
              "Working with " .. what),
            T("Режим работы с объектом включён. Диктуйте. Чтобы закончить, скажите: заканчиваем работу с объектом", "Listening"))
  end
end

local function obj_new_document(app)
  if app == "notepad" then
    if not jarvis.system.window_exists("Notepad") then
      launch("notepad.exe", {})
      jarvis.sleep(1200)
    end
    if WIN and not jarvis.system.window_exists("Notepad") then
      obj_say(T("Не удалось открыть Блокнот", "Could not open Notepad"))
      return false
    end
    obj_enter_typing(app, "")
    return true
  end
  local ok, _, err = office(app, "new")
  if not ok then obj_say(tostring(err)); return false end
  local ok2, info = obj_info(app)
  obj_enter_typing(app, ok2 and info.name or "", ok2 and info.path or nil)
  return true
end

-- the program is open: use its document, or ask which one to use
local function obj_begin(app)
  jarvis.state.set("obj_app", app)
  local ok, info, err = obj_info(app)
  if not ok then obj_stop(tostring(err)); return end
  if info.open then
    obj_enter_typing(app, info.name, info.path)
  else
    jarvis.session.start("object", 900)
    obj_step("doc_ask")
    obj_say(T("С " .. OBJ_NAMES[app] .. " открыт без документа. Создать новый документ или открыть существующий?", "Create a new document or open an existing one?"),
            T("Создать новый документ или открыть существующий?", "Create a new document or open an existing one?"))
  end
end

local function obj_app_from(phrase, tokens)
  local found = jarvis.apps.resolve(phrase)
  if found and found.office then return found.office end
  return target_from(tokens)
end

-- ---- stop / pause / continue (the state machine keeps the object, the program and the document stay open) ----
-- "стоп", "пауза": the dictation ends, nothing is closed, saved or deleted. After "стоп" Jarvis asks "Продолжим?".
local function obj_pause(pause_only)
  local reply, paused = jarvis.object.stop(pause_only)
  jarvis.object.note("user", pause_only and "пауза" or "стоп")
  if not paused then
    obj_say(reply, reply)
    return
  end
  if pause_only then
    jarvis.session.finish()
  else
    jarvis.session.start("object_resume", 120)
    jarvis.session.set_data("ask")
  end
  obj_say(reply, reply)
end

-- the object is gone (the program or the document was closed during the work): say so, offer to restore, never type elsewhere
local function obj_lost(why, offer)
  jarvis.object.lost(why)
  local info = jarvis.object.info()
  if info.state ~= "ERROR_RECOVERY" then jarvis.object.app_closed() end
  jarvis.session.start("object_resume", 180)
  jarvis.session.set_data(offer and "reopen" or "gone")
  local msg = why
  if offer then msg = msg .. T(" Открыть его снова? Скажите да или нет.", " Open it again?") end
  obj_say(msg, msg)
end

-- "продолжим с объектом": the program, the document and the window are checked again; only then the work goes on
local function obj_resume()
  local info = jarvis.object.info()
  local app, name = info.app, info.name
  if app ~= "word" and app ~= "excel" and app ~= "notepad" then
    jarvis.session.finish()
    obj_say(T("Нет работы, которую можно продолжить.", "Nothing to continue."))
    return
  end
  if app == "notepad" then
    if WIN and not jarvis.system.window_exists("Notepad") then
      return obj_lost(T("Блокнот закрыт.", "Notepad is closed."), false)
    end
  else
    local ok, di, err = obj_info(app)
    if not ok then
      return obj_lost(tostring(err) .. ".", false)
    end
    if not di.open then
      return obj_lost(T("Документ «" .. name .. "» закрыт.", "The document is closed."), info.path ~= "")
    end
    if name ~= "" and di.name ~= name then
      -- another document is active: the text must never go there. Stay paused and say what is needed
      jarvis.session.finish()
      local m = T("Сейчас активен другой документ «" .. di.name .. "». Нужен «" .. name .. "». Переключитесь на него и скажите «продолжим с объектом».", "Another document is active.")
      jarvis.object.note("jarvis", m)
      return say(OBJ_TITLE, m, m)
    end
  end
  local ok, why = jarvis.object.resume()
  if not ok then
    jarvis.session.finish()
    return obj_say(tostring(why))
  end
  jarvis.state.set("dict_last", "")
  jarvis.state.set("obj_app", app)
  jarvis.state.set("obj_name", name)
  obj_step("typing")
  local what = app == "notepad" and T("Блокнот", "Notepad") or ("«" .. name .. "»")
  obj_say(T("Продолжаем работу с " .. what .. ". Диктуйте.", "Going on with " .. what), T("Продолжаем. Диктуйте", "Going on"))
end

-- ---- the AI at work on the text of the document (docai.rs): the script reads and writes, Rust asks the AI ----
local AI_LANGS = { { "английск", "английский" }, { "русск", "русский" }, { "узбекск", "узбекский" }, { "немецк", "немецкий" }, { "французск", "французский" },
                  { "испанск", "испанский" }, { "турецк", "турецкий" }, { "итальянск", "итальянский" } }

-- a short whole phrase only: a long one is the dictated text
local function doc_ai_intent(tokens)
  local n = #tokens
  if n == 0 or n > 9 then return nil end
  local function has(stems) return stems_in(tokens, stems) end
  if has({ "исправ", "поправ", "проверь" }) and has({ "ошибк", "орфограф", "грамматик", "текст", "документ" }) then return "fix" end
  if has({ "перепиш", "сдела", "измен" }) and has({ "официальн", "формальн", "делов" }) then return "formal" end
  if has({ "переведи", "перевед", "перевод" }) then return "translate" end
  if has({ "сократи", "укороти", "короче", "покороче" }) then return "shorten" end
  if has({ "вступлен" }) and has({ "напиши", "добавь", "сделай", "составь" }) then return "intro" end
  if has({ "объявлени" }) and has({ "напиши", "составь", "сделай", "добавь" }) then return "announce" end
  return nil
end

local function obj_doc_ai(kind, phrase, tokens)
  local instr = phrase
  if kind == "translate" then
    local lang
    for _, l in ipairs(AI_LANGS) do if stems_in(tokens, { l[1] }) then lang = l[2] end end
    if not lang then return obj_say(T("На какой язык перевести? Например: «переведи на английский».", "Which language?")) end
    instr = "Переведи на " .. lang .. " язык"
  end
  local doc_b64 = ""
  if kind ~= "intro" and kind ~= "announce" then
    local ok, out, err, code = office("word", "read")
    if not ok then
      if code == "NOAPP" or code == "NODOC" then return obj_lost(tostring(err), jarvis.object.info().path ~= "") end
      return obj_say(tostring(err))
    end
    doc_b64 = out
  end
  local started, why = jarvis.object.doc_ai(kind, instr, doc_b64)
  if not started then return obj_say(tostring(why)) end
  obj_step("ai")
  obj_say(T("Работаю над текстом. Скажите «стоп», чтобы отменить: документ не изменится.", "Working on the text."), T("Работаю над текстом. Скажите стоп, чтобы отменить", "Working"))
end

-- the answer of the AI has come (the internal phrase of docai.rs): insert it, or ask before replacing the text
local function obj_ai_done()
  local r = jarvis.object.doc_ai_peek()
  if not r then return end
  if not r.ok then
    jarvis.object.doc_ai_drop()
    jarvis.object.recovered()
    obj_step("typing")
    return obj_say(tostring(r.error) .. T(" Документ не тронут. Продолжайте диктовать или повторите просьбу позже.", " The document is untouched."),
                   T("ИИ не ответил. Документ не тронут", "The AI did not answer"))
  end
  if r.replaces then
    obj_step("confirm_replace")
    return obj_say(T("Готов заменить весь текст документа новым вариантом (" .. r.chars .. " символов). Заменить? Скажите да или нет.", "Replace the text?"),
                   T("Готов заменить текст документа. Заменить? Скажите да или нет", "Replace the text?"))
  end
  -- an addition at the cursor
  local ok, _, err, code = office("word", "text", r.text_b64)
  if not ok then
    jarvis.object.doc_ai_drop()
    if code == "NOAPP" or code == "NODOC" then return obj_lost(tostring(err), jarvis.object.info().path ~= "") end
    obj_step("typing")
    return obj_say(tostring(err))
  end
  -- the fact of the insertion: the text is read back from the document
  local rok, rout = office("word", "read")
  local verified = rok and jarvis.object.b64_contains(rout, r.text_b64)
  jarvis.object.changed()
  jarvis.object.doc_ai_drop()
  obj_step("typing")
  if verified then
    obj_say(T("Вставил текст в документ (" .. r.chars .. " символов), проверил чтением. Он ещё не сохранён.", "Inserted and verified."), T("Вставил текст в документ", "Inserted"))
  else
    obj_say(T("Текст отправлен в документ, но подтвердить вставку чтением не удалось. Проверьте документ.", "Sent, but could not verify."), T("Проверьте документ: вставку не удалось подтвердить", "Check the document"))
  end
end

-- «да» / «нет» to «Заменить?»: written only when the document still has the text the AI was given; the result is read back
local function obj_replace_answer(a)
  local r = jarvis.object.doc_ai_peek()
  if not r or not r.ok then
    obj_step("typing")
    return obj_say(T("Нечего заменять.", "Nothing to replace."))
  end
  if a.yes and not a.no then
    local ok, out, err, code = office("word", "replace", r.text_b64, r.old_hash)
    jarvis.object.doc_ai_drop()
    if not ok then
      if code == "NOAPP" or code == "NODOC" then return obj_lost(tostring(err), jarvis.object.info().path ~= "") end
      obj_step("typing")
      return obj_say(tostring(err), tostring(err))
    end
    obj_step("typing")
    jarvis.object.changed()
    if out == jarvis.object.text_hash(r.text_b64) then
      return obj_say(T("Готово: текст заменён и проверен чтением. Если не понравилось, скажите «отмени последнее действие». Документ ещё не сохранён.", "Replaced and verified."),
                     T("Готово, текст заменён", "Replaced"))
    end
    return obj_say(T("Текст заменён, но проверка чтением не совпала. Проверьте документ.", "Replaced, but the check did not match."), T("Проверьте документ: проверка не совпала", "Check the document"))
  end
  jarvis.object.doc_ai_drop()
  obj_step("typing")
  obj_say(T("Хорошо, ничего не заменил.", "Not replaced."), T("Ничего не заменил", "Not replaced"))
end

-- ---- formatting phrases of Word ----
local function word_format(tokens)
  local n = #tokens
  if n == 0 or n > 5 then return nil end
  local has = function(stems) return stems_in(tokens, stems) end
  if has({ "отмени" }) and (has({ "последн", "действ" }) or n <= 2) then return "undo", nil end
  local on = has({ "включ", "сдела", "поставь", "добавь" })
  local off = has({ "выключ", "убери", "отключ", "сними" })
  if has({ "жирн" }) and (on or off) then return off and "bold_off" or "bold_on", nil end
  if has({ "курсив" }) and (on or off) then return off and "italic_off" or "italic_on", nil end
  if has({ "подчерк", "подчёрк" }) and (on or off) then return off and "underline_off" or "underline_on", nil end
  if has({ "по центру", "центр" }) and (on or has({ "выровн" })) then return "center", nil end
  if (has({ "по лев" }) or (has({ "лев" }) and has({ "край", "кра" }))) and (on or has({ "выровн" })) then return "left", nil end
  if (has({ "по прав" }) or (has({ "прав" }) and has({ "край", "кра" }))) and (on or has({ "выровн" })) then return "right", nil end
  if has({ "по ширин" }) and (on or has({ "выровн" })) then return "justify", nil end
  if has({ "заголов" }) and on then return (has({ "втор", "2" }) and "h2" or "h1"), nil end
  if has({ "обычн" }) and (on or has({ "текст", "стиль" })) then return "normal", nil end
  if has({ "нумерован" }) and (on or has({ "список" })) then return "numbers", nil end
  if has({ "маркиров", "список" }) and on then return "bullets", nil end
  if has({ "размер" }) and (on or has({ "шрифт" })) then
    local num = num_at(tokens, 1)
    for i = 1, n do num = num or (num_at(tokens, i)) end
    if num and num >= 6 and num <= 96 then return "size", tostring(math.floor(num)) end
  end
  return nil
end

local FORMAT_DONE = { bold_on = "жирный включён", bold_off = "жирный выключен", italic_on = "курсив включён", italic_off = "курсив выключен",
  underline_on = "подчёркивание включено", underline_off = "подчёркивание выключено", left = "по левому краю", center = "по центру", right = "по правому краю",
  justify = "по ширине", h1 = "заголовок", h2 = "заголовок 2", normal = "обычный текст", numbers = "нумерованный список", bullets = "маркированный список",
  undo = "последнее действие отменено", size = "размер шрифта изменён" }

local function looks_like_cell_command(tokens)
  local first = tokens[1] or ""
  if first:find("^ячейк") then return true end
  local col = parse_ref(tokens, 1)
  if col and tokens[2] then
    for i = 2, math.min(#tokens, 3) do
      if WRITE[tokens[i]] or FORMULA[tokens[i]] then return true end
    end
  end
  return false
end

-- one dictated phrase while the work mode is on
local obj_do_save -- (defined below with the saving code; "сохрани" can be said while dictating)

-- "сохрани", "сохрани и закрой", "сохрани документ и закрой ворд": ONLY a short phrase made of these words. Any other
-- sentence is dictated text ("нужно сохранить данные в базе" stays in the document).
local SAVE_WORDS = set_of({ "сохрани", "сохранить", "сохраняй", "сохраняем", "и", "документ", "файл", "закрой", "закрыть", "закрывай", "закрываем",
  "ворд", "word", "эксель", "excel", "блокнот", "программу", "приложение", "окно", "пожалуйста", "все", "всё", "это", "его", "тоже", "потом" })
local function save_intent(tokens)
  local n = #tokens
  if n == 0 or n > 6 then return nil end
  local save, close = false, false
  for _, w in ipairs(tokens) do
    if not SAVE_WORDS[w] then return nil end
    -- (the letters are two bytes each: the stem length is taken in bytes)
    if w:sub(1, #"сохран") == "сохран" then save = true end
    if w:sub(1, #"закр") == "закр" then close = true end
  end
  if not save then return nil end
  return { close = close }
end

local function obj_type(phrase)
  local tokens = tokens_of(phrase)
  local app = obj_app()
  -- "стоп", "пауза": only a whole short phrase, a longer one is the dictated text (objects.rs decides by the state)
  local ctl = jarvis.object.classify(phrase)
  if ctl == "stop" or ctl == "cancel" then return obj_pause(false) end
  if ctl == "pause" then return obj_pause(true) end
  if app == "word" then
    local kind = doc_ai_intent(tokens)
    if kind then return obj_doc_ai(kind, phrase, tokens) end
  end
  local want = save_intent(tokens)
  if want then
    jarvis.state.set("obj_close_after", want.close and "1" or "")
    return obj_do_save()
  end
  if is_object_end(tokens) then
    obj_step("ask_end")
    obj_say(T("Сохранить документ, удалить его или продолжить редактирование?", "Save the document, delete it or go on editing?"))
    return
  end

  if app == "word" then
    local cmd, arg = word_format(tokens)
    if cmd then
      local ok, _, err = office("word", "format", cmd, arg)
      say_quiet(ok and ("⚙ " .. (FORMAT_DONE[cmd] or cmd)) or tostring(err))
      return
    end
    local text = format_dictation(phrase)
    if text == "" then return end
    local ok, _, err, code = office("word", "text", b64(text))
    if ok then jarvis.out("✎ " .. text:gsub("\n", " ↵ ")) else
      jarvis.state.set("dict_last", "")
      if code == "NOAPP" or code == "NODOC" then return obj_lost(tostring(err), jarvis.object.info().path ~= "") end
      obj_say(tostring(err), tostring(err))
    end

  elseif app == "excel" then
    if looks_like_cell_command(tokens) then
      local ok, msg = excel_command(phrase)
      if ok then say_quiet(T("Ячейка: ", "Cell: ") .. msg) else obj_say(tostring(msg), tostring(msg)) end
      return
    end
    local joined = table.concat(tokens, " ")
    local num = (#tokens > 0) and jarvis.parse.number(joined) or nil
    local ok, out, err, code
    if num ~= nil then
      ok, out, err, code = office("excel", "active", b64(fmt_number(num)), "n")
    else
      ok, out, err, code = office("excel", "active", b64(cap_first(joined)), "t")
    end
    if ok then say_quiet(T("Ячейка ", "Cell ") .. tostring(out) .. " = " .. joined) else
      if code == "NOAPP" or code == "NODOC" then return obj_lost(tostring(err), jarvis.object.info().path ~= "") end
      obj_say(tostring(err), tostring(err))
    end

  else -- Notepad: only the Notepad window, never another program
    local text = format_dictation(phrase)
    if text == "" then return end
    local ok, err = jarvis.system.type_text(text, "Notepad")
    if ok then jarvis.out("✎ " .. text:gsub("\n", " ↵ ")) else
      jarvis.state.set("dict_last", "")
      local m = T("Блокнот закрыт или не найден. Текст не записан.", "Notepad is closed. Nothing was written.")
      obj_lost(m, false)
    end
  end
end

-- ---- saving ----
local PLACES = { { stems = { "рабоч", "стол" }, dir = "@Desktop", name = "рабочий стол" }, { stems = { "загрузк" }, dir = "@Downloads", name = "Загрузки" } }
local NAME_NOISE = set_of({ "назови", "назвать", "название", "имя", "как", "пусть", "будет", "файл", "сохрани", "сохранить", "документ", "в", "на", "умолчанию",
  "папку", "папке", "папка", "папки", "рабочем", "рабочий", "столе", "стол", "загрузки", "загрузках", "документы", "документах", "и", "с", "именем", "под", "названием", "пожалуйста", "там" })

-- "отчёт по информатике на рабочем столе" -> "@Desktop\\Отчёт по информатике.docx", "рабочий стол"
local function save_target(phrase, app)
  local tokens = tokens_of(phrase)
  local dir, where = "@Documents", "Документы"
  for _, pl in ipairs(PLACES) do
    if stems_in(tokens, pl.stems) then dir, where = pl.dir, pl.name end
  end
  -- the name keeps the letters as spoken ("отчёт", with ё); only the comparison with the noise words is normalized
  local words = {}
  for w in phrase:gmatch("%S+") do
    local bare = (w:gsub("[%.,!%?;:]+", "")):gsub("ё", "е")
    if bare ~= "" and not NAME_NOISE[bare] and not (bare:sub(1, 6) == "рабоч") and not (bare:sub(1, 7) == "загрузк") then words[#words + 1] = (w:gsub("[%.,!%?;:]+", "")) end
  end
  local name = table.concat(words, " ")
  if name == "" or name == "по" or stems_in(tokens, { "любое", "любой", "неважно", "угодно", "умолчан" }) then
    local stamp = "Документ"
    local okd, d = pcall(os.date, "%Y-%m-%d %H-%M")
    if okd and d then stamp = stamp .. " " .. d end
    name = stamp
  else
    name = cap_first(name)
  end
  name = name:gsub('[\\\\/:*?"<>|]', ""):gsub("%s+$", "")
  return dir .. "\\" .. name .. "." .. OBJ_EXT[app], where, name
end

-- the program is asked to close the way its own cross does (no /f): the document is saved already, so nothing is lost
local CLOSE_EXE = { word = "WINWORD.EXE", excel = "EXCEL.EXE", notepad = "notepad.exe" }

local function obj_saved_ok(app, path)
  jarvis.object.saved(path)
  local close = jarvis.state.get("obj_close_after") == "1"
  jarvis.state.set("obj_close_after", "")
  if close and CLOSE_EXE[app] then
    obj_stop(T("Документ сохранён: " .. path .. ". Закрываю " .. OBJ_NAMES[app] .. ".", "Saved: " .. path .. ". Closing."),
             T("Документ сохранён. Закрываю программу", "The document is saved. Closing the program"))
    run("taskkill", { "/im", CLOSE_EXE[app] })
    return
  end
  obj_stop(T("Документ сохранён: " .. path .. ". Режим работы с объектом завершён.", "Saved: " .. path),
           T("Документ сохранён. Работа с объектом завершена", "The document is saved. Object mode is finished"))
end

-- one line is added to the document before it is saved: when and by whom it was made (once per document)
local function obj_stamp(app)
  if jarvis.state.get("obj_stamped") == "1" then return end
  local okd, d = pcall(os.date, "%d.%m.%Y %H:%M")
  if not okd or not d then return end
  local line = T("Создано в Джарвисе: ", "Made with Jarvis: ") .. d
  if app == "word" then
    local ok = office("word", "text", b64("\n\n" .. line))
    if ok then jarvis.state.set("obj_stamped", "1") end
  elseif app == "notepad" then
    local ok = jarvis.system.type_text("\n\n" .. line, "Notepad")
    if ok then jarvis.state.set("obj_stamped", "1") end
  end
end

function obj_do_save()
  local app = obj_app()
  obj_stamp(app)
  if app == "notepad" then
    local ok, info = obj_info("notepad")
    if not ok or not info.open then
      obj_say(T("Блокнот закрыт: сохранять нечего.", "Notepad is closed."), T("Блокнот закрыт, сохранять нечего", "Notepad is closed"))
      return
    end
    local title = info.name or ""
    if title:find("Безымянный") or title:find("Untitled") then
      obj_step("save_name")
      obj_say(T("Как назвать файл и куда его положить? Например: «заметки на рабочем столе». По умолчанию — папка «Документы».", "Name and place?"),
              T("Как назвать файл и где сохранить? По умолчанию в документы", "Name and place?"))
      return
    end
    local ok2, out, err = office("notepad", "save", b64(""))
    if ok2 then obj_saved_ok(app, T("файл в Блокноте", "the file")) else obj_say(tostring(err)) end
    return
  end
  local ok, out, err, code = office(app, "save")
  if ok then obj_saved_ok(app, out); return end
  if code == "NEEDNAME" then
    obj_step("save_name")
    obj_say(T("Как назвать документ и куда его положить? Например: «отчёт по информатике на рабочем столе». По умолчанию — папка «Документы».", "Name and place?"),
            T("Как назвать документ и где сохранить? По умолчанию в документы", "Name and place?"))
  else
    obj_say(tostring(err), tostring(err))
    obj_step("ask_end")
  end
end

local function obj_save_as(rel, overwrite)
  local app = obj_app()
  local action = app == "notepad" and "save" or "saveas"
  local ok, out, err, code
  if app == "notepad" then
    ok, out, err, code = office("notepad", "save", b64(rel), overwrite and "overwrite" or nil)
  else
    ok, out, err, code = office(app, "saveas", b64(rel), overwrite and "overwrite" or nil)
  end
  if ok then
    local path = (out ~= "" and out) or rel
    jarvis.state.set("obj_pending", "")
    obj_saved_ok(app, path)
    return
  end
  if code == "EXISTS" then
    jarvis.state.set("obj_pending", rel)
    obj_step("overwrite")
    local shown = rel:gsub("^@%w+\\", "")
    obj_say(T("Файл «" .. shown .. "» уже существует. Заменить его? Скажите да или нет.", "The file exists. Replace it?"),
            T("Такой файл уже есть. Заменить его? Скажите да или нет", "That file exists. Replace it?"))
  else
    obj_say(tostring(err), tostring(err))
    obj_step("ask_end")
  end
end

-- ---- deleting: only after a clear "да" ----
local function obj_ask_delete()
  local app = obj_app()
  local msg
  if app == "notepad" then
    msg = T("Окно Блокнота будет закрыто без сохранения: весь несохранённый текст пропадёт. Закрыть без сохранения? Скажите да или нет.", "Notepad will be closed without saving. Sure?")
  else
    local ok, info = obj_info(app)
    local name = ok and info.open and info.name or jarvis.state.get("obj_name") or ""
    if ok and info.open and info.has_path then
      msg = T("Документ «" .. name .. "» будет закрыт без сохранения, а его файл «" .. info.path .. "» перемещён в корзину. Несохранённые изменения пропадут. Удалить? Скажите да или нет.", "Delete?")
    else
      msg = T("Документ «" .. name .. "» ещё нигде не сохранён: он будет закрыт, и всё написанное пропадёт. Удалить? Скажите да или нет.", "Delete?")
    end
  end
  obj_step("confirm_delete")
  obj_say(msg, msg)
end

local function obj_do_delete()
  local app = obj_app()
  local ok, out, err = office(app, "discard")
  if not ok then
    obj_say(tostring(err), tostring(err))
    obj_step("ask_end")
    return
  end
  local recycled = false
  if app ~= "notepad" and out ~= "" then
    local ok2, res = office("file", "recycle", b64(out))
    recycled = ok2 and res == "recycled"
  end
  local text
  if app == "notepad" then
    text = out == "asking" and T("Блокнот спрашивает о сохранении: выберите «Не сохранять» в его окне.", "Notepad asks about saving: choose Don't save.") or T("Блокнот закрыт без сохранения.", "Notepad closed without saving.")
  else
    text = recycled and T("Документ закрыт без сохранения, файл перемещён в корзину.", "Closed, the file is in the recycle bin.") or T("Документ закрыт без сохранения.", "Closed without saving.")
  end
  obj_stop(text .. T(" Режим работы с объектом завершён.", " Object mode is finished."), text)
end

-- one phrase in any step of the work mode
local function obj_handle(phrase)
  local tokens = tokens_of(phrase)
  local step = jarvis.session.data()
  local a = answer_of(tokens)
  local app = obj_app()

  if step == "typing" then
    return obj_type(phrase)

  elseif step == "ai" then
    -- the AI is working: nothing is typed into the document meanwhile
    local ctl = jarvis.object.classify(phrase)
    if ctl == "stop" or ctl == "cancel" then return obj_pause(false) end
    if ctl == "pause" then return obj_pause(true) end
    obj_say(T("Я ещё работаю над текстом. Скажите «стоп», чтобы отменить.", "Still working. Say stop to cancel."), T("Ещё работаю над текстом. Скажите стоп, чтобы отменить", "Still working"))

  elseif step == "confirm_replace" then
    local ctl = jarvis.object.classify(phrase)
    if ctl == "stop" or ctl == "pause" then jarvis.object.doc_ai_drop(); return obj_pause(ctl == "pause") end
    obj_replace_answer(a)

  elseif step == "offer" then
    -- "Word открыт. Работаем с этим объектом?": да = a new document and dictation; нет = just the program; anything else
    -- is an ordinary command (the question is dropped)
    if a.no or a.cancel then return obj_stop(T("Хорошо, программа открыта, больше ничего не делаю.", "Okay, nothing more."), T("Хорошо", "Okay")) end
    -- "начинаем работу с объектом" instead of "да": the usual way (the document is used, or asked for)
    if is_object_start(tokens) then return obj_begin(app) end
    if a.yes then
      local ok, info = obj_info(app)
      if ok and info.open then return obj_enter_typing(app, info.name, info.path) end
      return obj_new_document(app)
    end
    jarvis.object.finish()
    jarvis.session.finish()
    jarvis.state.set("obj_app", "")
    return jarvis.app.run_text(phrase)

  elseif step == "app_ask" then
    if a.cancel then return obj_stop(T("Хорошо, режим работы с объектом не включён.", "Okay, cancelled.")) end
    local target = obj_app_from(phrase, tokens)
    if target ~= "word" and target ~= "excel" and target ~= "notepad" then
      return obj_say(T("С чем работаем: Word, Excel или Блокнот?", "Word, Excel or Notepad?"))
    end
    return obj_begin(target)

  elseif step == "doc_ask" then
    if a.cancel then return obj_stop(T("Хорошо, режим работы с объектом не включён.", "Okay, cancelled.")) end
    if a.new and not a.existing then
      obj_new_document(app)
    elseif a.existing then
      obj_step("doc_name")
      obj_say(T("Как называется файл? Я поищу его в «Документах», на рабочем столе и в загрузках.", "What is the file called?"),
              T("Как называется файл?", "What is the file called?"))
    else
      obj_unclear(T("Скажите: «новый» — создать документ, «существующий» — открыть готовый. Или «отмена».", "New or existing?"))
    end

  elseif step == "doc_name" then
    if a.cancel then return obj_stop(T("Хорошо, режим работы с объектом не включён.", "Okay, cancelled.")) end
    if a.new and #tokens <= 2 then return obj_new_document(app) end
    local words = {}
    for _, w in ipairs(tokens) do
      if not NAME_NOISE[w] and w ~= "открой" and w ~= "найди" and w ~= "называется" and w ~= "файл" then words[#words + 1] = w end
    end
    if #words == 0 then return obj_say(T("Назовите имя файла.", "Say the file name.")) end
    local ok, path, err = office("file", "find", b64(table.concat(words, " ")), OBJ_FIND_EXT[app])
    if not ok then return obj_say(tostring(err)) end
    if path == "" then
      return obj_say(T("Файл «" .. table.concat(words, " ") .. "» не найден. Назовите имя ещё раз, скажите «новый» или «отмена».", "Not found."))
    end
    if app == "notepad" then
      launch("notepad.exe", { path })
      jarvis.sleep(1200)
      return obj_enter_typing(app, "")
    end
    local ok2, _, err2 = office(app, "open", b64(path))
    if not ok2 then return obj_say(tostring(err2)) end
    local ok3, info = obj_info(app)
    obj_enter_typing(app, ok3 and info.open and info.name or "", ok3 and info.open and info.path or nil)

  elseif step == "ask_end" then
    if a.continue or a.cancel then
      obj_step("typing")
      return obj_say(T("Продолжаем редактирование. Диктуйте.", "Go on."), T("Продолжаем. Диктуйте", "Go on"))
    elseif a.save and not a.delete then
      jarvis.state.set("obj_close_after", stems_in(tokens, { "закр" }) and "1" or "")
      return obj_do_save()
    elseif a.delete and not a.save then
      return obj_ask_delete()
    end
    obj_unclear(T("Скажите: «сохранить», «удалить» или «продолжить». «Отмена» вернёт вас к документу.", "Save, delete or continue?"),
                T("Сохранить документ, удалить его или продолжить редактирование?", "Save, delete or continue?"))

  elseif step == "save_name" then
    if a.cancel then
      obj_step("typing")
      return obj_say(T("Сохранение отменено. Продолжаем редактирование.", "Cancelled."), T("Отменил. Продолжаем редактирование", "Cancelled"))
    end
    local rel, where, name = save_target(phrase, app)
    obj_say(T("Сохраняю «" .. name .. "» в «" .. where .. "»…", "Saving…"), T("Сохраняю", "Saving"))
    obj_save_as(rel, false)

  elseif step == "overwrite" then
    if a.yes and not a.no then
      return obj_save_as(jarvis.state.get("obj_pending") or "", true)
    end
    obj_step("save_name")
    obj_say(T("Хорошо, не заменяю. Назовите другое имя или скажите «отмена».", "Not replacing. Another name?"), T("Не заменяю. Назовите другое имя", "Another name?"))

  elseif step == "confirm_delete" then
    if (a.yes or a.delete) and not a.no and not a.cancel then
      return obj_do_delete()
    end
    obj_step("typing")
    obj_say(T("Хорошо, ничего не удаляю. Продолжаем редактирование.", "Not deleting."), T("Хорошо, ничего не удаляю. Продолжаем редактирование", "Not deleting"))

  else
    obj_stop()
  end
end

-- "начинаем работу с объектом"
local function obj_start(phrase)
  local tokens = tokens_of(phrase)
  local app = obj_app_from(phrase, tokens)
  if app ~= "word" and app ~= "excel" and app ~= "notepad" then
    local last = jarvis.state.get("office_app")
    if last == "word" or last == "excel" or last == "notepad" then app = last end
  end
  if app ~= "word" and app ~= "excel" and app ~= "notepad" then
    jarvis.session.start("object", 300)
    obj_step("app_ask")
    return obj_say(T("С каким объектом работаем: Word, Excel или Блокнот?", "Word, Excel or Notepad?"), T("С каким объектом работаем: ворд, эксель или блокнот?", "Word, Excel or Notepad?"))
  end
  obj_begin(app)
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

if id == "assistant_object_start" then
  jarvis.state.set("obj_name", "")
  obj_start(phrase)

elseif id == "assistant_object_ai_done" then
  obj_ai_done()

elseif id == "assistant_object_resume" then
  local st = jarvis.object.state()
  if st == "PAUSED" then
    obj_resume()
  elseif st == "ERROR_RECOVERY" then
    obj_lost(jarvis.object.info().error ~= "" and jarvis.object.info().error or T("Объект потерян.", "The object is lost."), jarvis.object.info().path ~= "")
  else
    say(OBJ_TITLE, T("Сейчас нет приостановленной работы с объектом.", "Nothing is paused."))
  end

elseif id == "assistant_office_dictate" then
  local tokens = tokens_of(phrase)
  local target = target_from(tokens) or jarvis.state.get("office_app")
  if target ~= "word" and target ~= "excel" and target ~= "notepad" then
    say(TITLE, T("Куда писать? Скажите: «пиши в Word», «пиши в блокноте» или «пиши в Excel».", "Where to write? Say: write in Word / Notepad / Excel."),
        T("Куда писать? В ворде, в блокноте или в экселе?", "Where to write? Word, Notepad or Excel?"))
    return
  end
  start_dictation(target)
  -- "пиши: привет, как дела?": the text after the command word is written at once (never run as a command)
  if target ~= "excel" then
    local skip = set_of({ "пиши", "пишите", "напиши", "диктуй", "диктуйте", "печатай", "в", "ворде", "ворд", "word", "блокноте", "блокнот", "notepad", "документе", "документ",
                          "текст", "пожалуйста", "начни", "писать", "диктовку", "диктовать", "буду", "давай", "режим", "диктовки" })
    -- the words as they were typed (with their commas and question marks)
    local rest, started = {}, false
    for w in phrase:gmatch("%S+") do
      local bare = (w:gsub("[%.,!%?;:]+", ""))
      if started or not skip[bare] then started = true; rest[#rest + 1] = w end
    end
    local text = table.concat(rest, " ")
    if text ~= "" then
      local formatted = format_dictation(text)
      if type_into(target, formatted) then jarvis.out("✎ " .. formatted:gsub("\n", " ↵ ")) end
    end
  end

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
  if phrase:find("objaidone", 1, true) then
    obj_ai_done()
  elseif kind == "file" then
    file_session(phrase)
  elseif kind == "app_choice" then
    -- the answer to "какую программу открыть: 1. … 2. …"
    local low = jarvis.text.lower(phrase)
    if low:find("отмен") or low:find("ничего") or low:find("не над") or low:find("cancel") then
      jarvis.session.finish()
      say(T("Программа", "Program"), T("Хорошо, ничего не открываю", "Okay, nothing is opened"))
      return
    end
    local name, err, exe, was_running = jarvis.apps.launch_pending(phrase)
    if name then
      jarvis.session.finish()
      say(T("Открываю", "Opening"), name, T("Открываю " .. name, "Opening " .. name), "program")
      confirm_started(name, exe, nil, was_running)
    elseif err == "не понял, какую программу выбрать" then
      -- not an answer: leave the question, the phrase is an ordinary command
      jarvis.session.finish()
      jarvis.app.run_text(phrase)
    else
      jarvis.session.finish()
      say(T("Программа", "Program"), T("Не удалось запустить: ", "Could not start: ") .. tostring(err))
    end
  elseif kind == "object" then
    local nm = jarvis.state.get("obj_name")
    local app = jarvis.state.get("obj_app")
    local step = jarvis.session.data()
    if nm and nm ~= "" and (app == "word" or app == "excel") and step ~= "doc_ask" and step ~= "doc_name" and step ~= "app_ask" then BOUND = nm end
    obj_handle(phrase)
  elseif kind == "object_resume" then
    -- the answer to "Продолжим?" (or to "Открыть его снова?" after the document was closed)
    local ctl = jarvis.object.classify(phrase)
    local step = jarvis.session.data()
    if step == "reopen" or step == "gone" then
      local yes = exact_in(tokens_of(phrase), { "да", "ага", "конечно", "давай", "открой" })
      local no = exact_in(tokens_of(phrase), { "нет", "не", "неа" }) or ctl == "stop" or ctl == "cancel"
      jarvis.session.finish()
      if not yes or step == "gone" then
        jarvis.object.finish()
        return obj_say(T("Хорошо, ничего не открываю. Работа с объектом завершена.", "Okay."))
      end
      local info = jarvis.object.info()
      local app = info.app
      local okp = office(app, "open", b64(info.path))
      if not okp then
        jarvis.object.finish()
        return obj_say(T("Не удалось открыть «" .. info.name .. "». Работа с объектом завершена.", "Could not open it."))
      end
      local ok3, di = obj_info(app)
      if not (ok3 and di.open) then
        jarvis.object.finish()
        return obj_say(T("Не удалось подтвердить, что документ открыт. Работа с объектом завершена.", "Not confirmed."))
      end
      jarvis.object.recovered()
      jarvis.object.release()
      return obj_enter_typing(app, di.name, di.path)
    end
    if ctl == "yes" or ctl == "resume" then
      return obj_resume()
    elseif ctl == "no" or ctl == "stop" then
      jarvis.object.decline_resume()
      jarvis.session.finish()
      return obj_say(T("Хорошо, документ остался открытым. Скажите «продолжим с объектом», когда понадобится.", "Okay, nothing is closed."),
                     T("Хорошо, ничего не закрываю", "Okay"))
    end
    -- not an answer: the question is left, the phrase is an ordinary command
    jarvis.session.finish()
    jarvis.app.run_text(phrase)
  elseif kind == "dictation" then
    local tokens = tokens_of(phrase)
    local target = jarvis.session.data()
    if is_object_start(tokens) then
      jarvis.session.finish()
      jarvis.state.set("obj_name", "")
      obj_start(phrase)
      return
    end
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
