-- keyboard shortcuts, mouse and windows: "новая вкладка", "выдели всё", "клик", "разверни окно" ...
-- The action goes to the window the user works in (jarvis.system.keys / mouse / window): when the Jarvis window is
-- in front, the window under it gets the focus first. Nothing is reported as done when it could not be done.

local id = jarvis.context.command_id
local ACTIONS = {
  web_tab_new = { "keys", "ctrl+t", "Новая вкладка", "New tab" },
  web_tab_close = { "keys", "ctrl+w", "Вкладка закрыта", "Tab closed" },
  web_tab_reopen = { "keys", "ctrl+shift+t", "Вкладка возвращена", "Tab reopened" },
  web_tab_next = { "keys", "ctrl+tab", "Следующая вкладка", "Next tab" },
  web_tab_prev = { "keys", "ctrl+shift+tab", "Предыдущая вкладка", "Previous tab" },
  web_back = { "keys", "alt+left", "Назад", "Back" },
  web_forward = { "keys", "alt+right", "Вперёд", "Forward" },
  web_refresh = { "keys", "f5", "Страница обновлена", "Page refreshed" },
  web_history = { "keys", "ctrl+h", "История браузера", "History" },
  web_downloads = { "keys", "ctrl+j", "Загрузки браузера", "Downloads" },
  web_bookmark = { "keys", "ctrl+d", "Добавлено в закладки", "Bookmarked" },
  web_incognito = { "keys", "ctrl+shift+n", "Окно инкогнито", "Incognito window" },
  web_window_new = { "keys", "ctrl+n", "Новое окно", "New window" },
  web_address = { "keys", "ctrl+l", "Адресная строка", "Address bar" },
  web_find = { "keys", "ctrl+f", "Поиск по странице", "Find on page" },
  key_undo = { "keys", "ctrl+z", "Отменено", "Undone" },
  key_redo = { "keys", "ctrl+y", "Повторено", "Redone" },
  key_select_all = { "keys", "ctrl+a", "Выделено всё", "Selected all" },
  key_copy = { "keys", "ctrl+c", "Скопировано", "Copied" },
  key_paste = { "keys", "ctrl+v", "Вставлено", "Pasted" },
  key_cut = { "keys", "ctrl+x", "Вырезано", "Cut" },
  key_page_top = { "keys", "ctrl+home", "Начало страницы", "Top of the page" },
  key_page_end = { "keys", "ctrl+end", "Конец страницы", "End of the page" },
  key_print = { "keys", "ctrl+p", "Печать", "Print" },
  key_save = { "keys", "ctrl+s", "Сохранено", "Saved" },
  key_clipboard = { "keys", "win+v", "История буфера обмена", "Clipboard history" },
  key_enter = { "keys", "enter", "Enter", "Enter" },
  key_escape = { "keys", "esc", "Esc", "Esc" },
  mouse_click = { "mouse", "click", "Клик", "Click" },
  mouse_double = { "mouse", "double", "Двойной клик", "Double click" },
  mouse_right = { "mouse", "right", "Правый клик", "Right click" },
  mouse_up = { "mouse", "up", "Курсор вверх", "Cursor up" },
  mouse_down = { "mouse", "down", "Курсор вниз", "Cursor down" },
  mouse_left = { "mouse", "left", "Курсор влево", "Cursor left" },
  mouse_right_move = { "mouse", "right_move", "Курсор вправо", "Cursor right" },
  mouse_scroll_up = { "mouse", "scroll_up", "Прокрутка вверх", "Scrolled up" },
  mouse_scroll_down = { "mouse", "scroll_down", "Прокрутка вниз", "Scrolled down" },
  win_switch = { "keys", "alt+tab", "Окно переключено", "Window switched" },
  win_maximize = { "window", "maximize", "Окно развёрнуто", "Window maximized" },
  win_minimize = { "window", "minimize", "Окно свёрнуто", "Window minimized" },
  win_restore = { "window", "restore", "Окно восстановлено", "Window restored" },
}

local a = ACTIONS[id]
if not a then
  say(T("Ввод", "Input"), T("Неизвестная команда ввода: ", "Unknown input command: ") .. tostring(id), false)
  return
end
local kind, arg, ru, en = a[1], a[2], a[3], a[4]
local ok, why
if kind == "keys" then
  ok, why = jarvis.system.keys(arg)
elseif kind == "mouse" then
  ok, why = jarvis.system.mouse(arg, nil)
else
  ok, why = jarvis.system.window(arg)
end
if ok then
  -- short and silent: a spoken reply after every key press would only get in the way
  jarvis.reply(T(ru, en))
else
  say(T("Не получилось", "Not done"), T(ru, en) .. ": " .. tostring(why), T("Не получилось", "Could not do it"))
end
