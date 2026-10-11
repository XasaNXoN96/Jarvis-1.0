-- find files on this computer: "найди на моём компьютере файл 123", "открой первый результат"

local id = jarvis.context.command_id

local ORDINALS = { ["перв"] = 1, ["втор"] = 2, ["трет"] = 3, ["четв"] = 4, ["пят"] = 5, ["шест"] = 6, ["седьм"] = 7, ["восьм"] = 8,
                   ["девят"] = 9, ["десят"] = 10, ["first"] = 1, ["second"] = 2, ["third"] = 3, ["fourth"] = 4, ["fifth"] = 5 }

local function wanted_index()
  for _, w in ipairs(words()) do
    for stem, n in pairs(ORDINALS) do
      if w:sub(1, #stem) == stem then return n end
    end
  end
  local n = jarvis.parse.number(phrase)
  if n and n >= 1 then return math.floor(n) end
  return 1
end

if id == "assistant_file_search" then
  local STOP = {
    "найди", "найти", "найдите", "поищи", "поиск", "искать", "ищи", "файл", "файлы", "файла", "папку", "папка", "папки", "документ",
    "на", "моем", "моём", "мой", "моя", "моих", "компьютере", "компьютер", "компе", "комп", "пк", "диске", "дисках",
    "названием", "названии", "именем", "имя", "называется", "называемый", "под", "с", "по", "который", "которая", "пожалуйста", "мне", "там", "где", "у", "меня",
    "find", "search", "for", "file", "files", "folder", "named", "called", "on", "my", "computer", "pc", "the", "a", "please", "me",
  }
  local query = strip_words(phrase, STOP)
  if query == "" then
    say(T("Поиск файла", "File search"), T("Что искать? Например: найди файл отчёт", "What to look for? e.g. find file report"))
    return
  end

  -- a name made only of numbers: "один два три" / "1 2 3" -> 123, "сто двадцать три" -> 123
  local tokens, all_numbers, all_single = {}, true, true
  for w in query:gmatch("%S+") do
    local n = jarvis.parse.number(w)
    tokens[#tokens + 1] = n
    if n == nil then all_numbers = false
    elseif n ~= math.floor(n) or n < 0 or n > 9 then all_single = false end
  end
  if all_numbers and #tokens > 0 then
    if all_single then
      local digits = {}
      for _, n in ipairs(tokens) do digits[#digits + 1] = tostring(math.floor(n)) end
      query = table.concat(digits)
    else
      local n = jarvis.parse.number(query)
      if n then query = tostring(math.floor(n)) end
    end
  end

  jarvis.files.search(query)
  say(T("Поиск файла", "File search"), T("Ищу «" .. query .. "»…", "Searching for \"" .. query .. "\"…"),
      T("Ищу " .. query, "Searching for " .. query))
  return
end

local list = jarvis.files.results()
if #list == 0 then
  say(T("Поиск файла", "File search"), T("Сначала найдите файл: «найди файл ...»", "Search first: \"find file ...\""),
      T("Сначала скажите, какой файл найти", "Tell me which file to find first"))
  return
end

local n = wanted_index()
local item = list[n]
if not item then
  say(T("Поиск файла", "File search"), T("Результата номер " .. n .. " нет (найдено " .. #list .. ")", "No result number " .. n .. " (found " .. #list .. ")"))
  return
end

if id == "assistant_file_folder" then
  -- show the file in Explorer (selected)
  launch("explorer.exe", { "/select," .. item.path:gsub("/", "\\") })
  say(T("Папка", "Folder"), item.dir, T("Показываю папку", "Showing the folder"))
else
  open(item.path)
  say(T("Открываю", "Opening"), item.name, T("Открываю " .. item.name, "Opening " .. item.name))
end
