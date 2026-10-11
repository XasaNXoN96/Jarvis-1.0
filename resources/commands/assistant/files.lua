-- find files on this computer: "найди файл report", "найди все PDF", "найди фотографии за октябрь",
-- "найди файл, содержащий текст ICTERA", "открой найденный файл", "покажи, где он лежит", "ещё", "ищи везде".
-- The phrase is understood in crates/jarvis-core/src/filesearch.rs (name, kind, place, dates, text inside).
-- Nothing is opened by itself: with several results the user says which one.

local id = jarvis.context.command_id
local TITLE = T("Поиск файла", "File search")

local ORDINALS = { ["перв"] = 1, ["втор"] = 2, ["трет"] = 3, ["четв"] = 4, ["пят"] = 5, ["шест"] = 6, ["седьм"] = 7, ["восьм"] = 8,
                   ["девят"] = 9, ["десят"] = 10, ["first"] = 1, ["second"] = 2, ["third"] = 3, ["fourth"] = 4, ["fifth"] = 5 }

-- the number said in the phrase ("второй", "номер 3"), or nil
local function said_index()
  for _, w in ipairs(words()) do
    for stem, n in pairs(ORDINALS) do
      if w:sub(1, #stem) == stem then return n end
    end
  end
  local n = jarvis.parse.number(phrase)
  if n and n >= 1 then return math.floor(n) end
  return nil
end

if id == "assistant_file_search" then
  local what, err = jarvis.files.search(jarvis.context.phrase or phrase)
  if not what then
    say(TITLE, err)
    return
  end
  say(TITLE, T("Ищу: ", "Searching: ") .. what .. "…", T("Ищу", "Searching"))
  return
end

if id == "assistant_file_cancel" then
  if jarvis.files.cancel() then
    if jarvis.session.kind() == "file" then jarvis.session.finish() end
    say(TITLE, T("Поиск остановлен", "Search stopped"))
  else
    say(TITLE, T("Сейчас ничего не ищу", "No search is running"))
  end
  return
end

if id == "assistant_file_widen" then
  local what, err = jarvis.files.widen()
  if not what then say(TITLE, err) return end
  if jarvis.session.kind() == "file" then jarvis.session.finish() end
  say(TITLE, T("Ищу на всём компьютере: ", "Searching the whole computer: ") .. what .. T(". Это может занять до двух минут.", ". It can take up to two minutes."),
      T("Ищу на всём компьютере", "Searching the whole computer"))
  return
end

local list = jarvis.files.results()
if #list == 0 then
  say(TITLE, T("Сначала найдите файл: «найди файл ...»", "Search first: \"find file ...\""),
      T("Сначала скажите, какой файл найти", "Tell me which file to find first"))
  return
end

if id == "assistant_file_more" then
  say(TITLE, jarvis.files.more(), false)
  jarvis.session.start("file", 60)
  return
end

-- the chosen result: said now, or chosen earlier in this conversation ("второй", "следующий")
local selected = jarvis.files.selected()

if id == "assistant_file_next" then
  local n = (selected or 0) + 1
  local item = list[n]
  if not item then
    say(TITLE, T("Это был последний результат (всего " .. #list .. ")", "That was the last result (" .. #list .. " in total)"))
    return
  end
  jarvis.files.select(n)
  jarvis.session.start("file", 60)
  jarvis.session.set_data(tostring(n))
  say(TITLE, n .. ". " .. item.name .. " — " .. item.dir .. T(". Открыть или показать папку?", ". Open it or show the folder?"),
      T("Номер " .. n .. ". Открыть?", "Number " .. n .. ". Open it?"))
  return
end

local n = said_index() or selected or (#list == 1 and 1 or nil)
if not n then
  -- several results and none chosen: never a random one
  jarvis.session.start("file", 60)
  jarvis.session.set_data("")
  say(TITLE, T("Найдено " .. #list .. ". Какой открыть? Скажите номер, например «второй».", "Found " .. #list .. ". Which one? Say the number."),
      T("Какой номер?", "Which number?"))
  return
end
local item = list[n]
if not item then
  say(TITLE, T("Результата номер " .. n .. " нет (найдено " .. #list .. ")", "No result number " .. n .. " (found " .. #list .. ")"))
  return
end
jarvis.files.select(n)

if id == "assistant_file_folder" then
  -- show the file in Explorer (selected)
  local shown, why = jarvis.files.reveal(item.path)
  if not shown then say(T("Папка", "Folder"), T("Не удалось показать: ", "Cannot show: ") .. tostring(why)); return end
  say(T("Папка", "Folder"), item.dir, T("Показываю папку", "Showing the folder"))
else
  if WIN and not jarvis.system.exists(item.path) then
    say(TITLE, T("Файла больше нет на месте: ", "The file is gone: ") .. item.name)
    return
  end
  local ok = pcall(open, item.path)
  if ok then say(T("Открываю", "Opening"), item.name, T("Открываю", "Opening"))
  else say(TITLE, T("Не удалось открыть ", "Could not open ") .. item.name .. T(" (нет программы для этого типа или нет доступа)", " (no program for this type or no access)")) end
end
