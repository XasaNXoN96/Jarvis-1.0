// QUICK HELP ON HOW TO ADD NEW LUA API MODULE
//
// # 1. DEFINE NEW API MODULE FILE

use mlua::{Lua, Table};
use crate::lua::error::LuaError;

pub fn register(lua: &Lua, jarvis: &Table) -> mlua::Result<()> {
    let mymodule = lua.create_table()?;
    
    // add functions
    let my_fn = lua.create_function(|_, arg: String| {
        // implementation
        Ok(format!("Result: {}", arg))
    })?;
    mymodule.set("my_function", my_fn)?;
    
    jarvis.set("mymodule", mymodule)?;
    
    Ok(())
}

// # 2. ADD NEW API MODULE TO mod.rs

pub mod mymodule;


// # 3. REGISTER NEW MODULE IN engine.rs
// in register_api()
api::mymodule::register(&self.lua, &jarvis)?;

// # 4. YOU CAN ALSO DEFINE ASYNC FUNCTIONS INSTEAD
let async_fn = lua.create_async_function(|_, url: String| async move {
    let response = reqwest::get(&url).await
        .map_err(|e| mlua::Error::runtime(e.to_string()))?;
    Ok(response.text().await.unwrap_or_default())
})?;