-- weather command using wttr.in API

local lang = jarvis.context.language

-- get saved city or use default
local city = jarvis.state.get("city") or "Moscow"

jarvis.log("info", "Fetching weather for: " .. city)

-- build URL
local url = "https://wttr.in/" .. city .. "?format=3&lang=" .. lang

-- make request
local response = jarvis.http.get(url)

if response.ok then
    jarvis.log("info", "Weather: " .. response.body)
    
    -- show notification
    local title = lang == "ru" and "Погода" or "Weather"
    jarvis.system.notify(title, response.body)
    
    jarvis.audio.play_ok()
else
    jarvis.log("error", "Failed to fetch weather: " .. (response.error or "unknown error"))
    jarvis.audio.play_error()
end

return { chain = false }