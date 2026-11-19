-- The morning briefing: normally the program recognises these phrases itself (digest.rs) before the command search.
-- This script runs only when the intent model matched a close variant: it hands the canonical phrase to the program.
-- (The canonical phrases are always recognised by digest.rs, so this cannot loop.)

local id = jarvis.context.command_id
local canonical = {
  digest_now = "утренняя сводка",
  digest_on = "включи утреннюю сводку",
  digest_off = "выключи утреннюю сводку",
}
jarvis.app.run_text(canonical[id] or "утренняя сводка")
