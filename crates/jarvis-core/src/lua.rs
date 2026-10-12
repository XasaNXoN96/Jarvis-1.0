mod engine;
mod sandbox;
mod error;
mod api;
pub use api::system::process_running;

mod structs;
pub use structs::*;

pub use engine::LuaEngine;
pub use sandbox::SandboxLevel;
pub use error::LuaError;

// timers saved by the previous run are restored at start-up
pub use api::timer::init as init_timers;
pub use api::timer::{cancel_one as cancel_timer, list_json as timers_json};
// what the scenarios of the "Конструктор" reuse (scenario.rs): the same functions the commands call
pub use api::system::{type_text, volume_control};
#[cfg(target_os = "windows")]
pub use api::system::{set_clipboard_text, shell_open};

use std::path::PathBuf;
use std::time::Duration;

#[cfg(test)]
mod tests;

#[cfg(all(test, unix))]
mod object_tests;

// Execute a Lua command script
pub fn execute(
    script_path: &PathBuf,
    context: CommandContext,
    sandbox: SandboxLevel,
    timeout: Duration,
) -> Result<CommandResult, LuaError> {
    let engine = LuaEngine::new(sandbox)?;
    engine.execute(script_path, context, timeout)
}