-- phone remote, from the PC to the phone page: "найди телефон", "останови сигнал телефона",
-- "отправь ссылку на телефон" (the link from the clipboard), "отправь на телефон <текст>" (or the clipboard).
-- The phone gets it only while the remote page is open there; a web page cannot ring a sleeping phone.

local id = jarvis.context.command_id

local function done(reply, err)
  if not reply then
    say(T("Телефон", "Phone"), T("Не отправлено: ", "Not sent: ") .. (err or "?"))
    return
  end
  if reply == "отправлено на телефон" then
    return true
  end
  -- sent, but the phone is not connected right now: say so instead of pretending
  say(T("Телефон", "Phone"), T("Отправлено, но ", "Sent, but ") .. reply)
  return false
end

if id == "assistant_phone_find" then
  local low = jarvis.text.lower(jarvis.context.phrase or "")
  if low:find("файл") or low:find("папк") or low:find("документ") or low:find("фото") then
    say(T("Телефон", "Phone"), T("Искать файлы на телефоне пульт не умеет: он может только подать сигнал, чтобы найти сам телефон.", "The remote cannot search files on the phone: it can only ring to find the phone itself."))
    return
  end
  local reply, err = jarvis.remote.send("ring")
  if done(reply, err) then
    say(T("Телефон", "Phone"), T("Телефон звонит. Скажите «останови сигнал», когда найдёте.", "The phone is ringing. Say \"stop the signal\" when you find it."))
  end
  return
end

if id == "assistant_phone_stop" then
  local reply, err = jarvis.remote.send("stop")
  if done(reply, err) then say(T("Телефон", "Phone"), T("Сигнал остановлен", "Signal stopped")) end
  return
end

local clip = ""
if jarvis.system.clipboard and jarvis.system.clipboard.get then
  clip = jarvis.system.clipboard.get() or ""
end
clip = clip:gsub("^%s+", ""):gsub("%s+$", "")

if id == "assistant_phone_link" then
  -- "открой ютуб на телефоне": a site of the catalogue; otherwise the link from the clipboard
  local skip = { ["открой"] = true, ["открыть"] = true, ["отправь"] = true, ["скинь"] = true, ["перешли"] = true, ["ссылку"] = true,
    ["ссылка"] = true, ["на"] = true, ["сайт"] = true, ["send"] = true, ["open"] = true, ["link"] = true, ["the"] = true, ["on"] = true, ["to"] = true, ["my"] = true }
  local name = {}
  for w in (jarvis.context.phrase or ""):gmatch("%S+") do
    local lw = jarvis.text.lower(w)
    if not (skip[lw] or lw:find("^телефон") or lw:find("^phone")) then name[#name + 1] = lw end
  end
  local site_url, site_name = nil, nil
  if #name > 0 then site_url, site_name = jarvis.sites.find(table.concat(name, " ")) end
  local link = site_url or clip:match("^(https?://%S+)$")
  if not link then
    say(T("Телефон", "Phone"), T("В буфере обмена нет ссылки. Скопируйте адрес страницы (Ctrl+L, Ctrl+C) и повторите.", "There is no link in the clipboard. Copy the page address (Ctrl+L, Ctrl+C) and try again."))
    return
  end
  local reply, err = jarvis.remote.send("link", link)
  if done(reply, err) then
    say(T("Телефон", "Phone"), site_name and (site_name .. T(": ссылка отправлена на телефон, нажмите «Открыть» на пульте", ": link sent to the phone, tap Open on the remote"))
      or T("Ссылка отправлена на телефон", "Link sent to the phone"))
  end
  return
end

-- assistant_phone_text: the words after "на телефон" (or between the verb and "на телефон"), else the clipboard
local original = jarvis.context.phrase or ""
local words = {}
for w in original:gmatch("%S+") do words[#words + 1] = w end
local skip = { ["отправь"] = true, ["отправить"] = true, ["скинь"] = true, ["перешли"] = true, ["пошли"] = true,
  ["send"] = true, ["текст"] = true, ["text"] = true, ["на"] = true, ["to"] = true, ["the"] = true, ["my"] = true, ["мой"] = true }
local rest = {}
local i = 1
while i <= #words do
  local lw = jarvis.text.lower(words[i])
  if lw:find("^телефон") or lw:find("^phone") then
    -- "на телефон" / "to phone" is not part of the text
  elseif #rest == 0 and skip[lw] then
    -- the leading verb
  elseif (lw == "на" or lw == "to") and words[i + 1] and (jarvis.text.lower(words[i + 1]):find("^телефон") or jarvis.text.lower(words[i + 1]):find("^phone")) then
    -- "на" right before "телефон"
  else
    rest[#rest + 1] = words[i]
  end
  i = i + 1
end
local text = table.concat(rest, " ")
local low = jarvis.text.lower(text)
if low == "" or low == "буфер" or low == "буфер обмена" or low == "из буфера" or low == "из буфера обмена" or low == "clipboard" then
  text = clip
end
if text == "" then
  say(T("Телефон", "Phone"), T("Нечего отправлять: скажите текст после «на телефон» или скопируйте его в буфер обмена.", "Nothing to send: say the text after \"to phone\" or copy it to the clipboard."))
  return
end
local reply, err = jarvis.remote.send("text", text)
if done(reply, err) then
  say(T("Телефон", "Phone"), T("Текст отправлен на телефон (" .. jarvis.text.len(text) .. " симв.)", "Text sent to the phone"),
      T("Отправил на телефон", "Sent to the phone"))
end
