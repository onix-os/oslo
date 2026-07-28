use crate::env::Environment;
use crate::error::{Result, ShellError};
use mlua::prelude::*;
use std::sync::{Arc, Mutex};

pub struct LuaEngine {
    lua: Lua,
    pub prompt_fn: Option<LuaFunction>,
    pub precmd_fn: Option<LuaFunction>,
    pub postcmd_fn: Option<LuaFunction>,
    pub cd_fn: Option<LuaFunction>,
}

impl Default for LuaEngine {
    fn default() -> Self {
        Self::new().expect("Failed to initialize Lua engine")
    }
}

impl LuaEngine {
    pub fn new() -> Result<Self> {
        let lua = Lua::new();

        Ok(Self {
            lua,
            prompt_fn: None,
            precmd_fn: None,
            postcmd_fn: None,
            cd_fn: None,
        })
    }

    pub fn setup_bindings(&self, env: Arc<Mutex<Environment>>) -> Result<()> {
        let globals = self.lua.globals();
        let rush_table = self.lua.create_table()?;

        // rush.exec(cmd_string)
        let env_exec = Arc::clone(&env);
        let exec_fn = self.lua.create_function(move |_, cmd_str: String| {
            let mut env_guard = env_exec.lock().unwrap();
            let ast = crate::parser::parse_bash_script(&cmd_str).map_err(|e| e.into_lua_err())?;
            let status = crate::exec::eval_command_list(&mut env_guard, &ast)
                .map_err(|e| e.into_lua_err())?;
            Ok(status)
        })?;
        rush_table.set("exec", exec_fn)?;

        // rush.get_var(name)
        let env_get = Arc::clone(&env);
        let get_var_fn = self.lua.create_function(move |_, name: String| {
            let env_guard = env_get.lock().unwrap();
            Ok(env_guard.get_param(&name))
        })?;
        rush_table.set("get_var", get_var_fn)?;

        // rush.set_var(name, val)
        let env_set = Arc::clone(&env);
        let set_var_fn = self
            .lua
            .create_function(move |_, (name, val): (String, String)| {
                let mut env_guard = env_set.lock().unwrap();
                env_guard.set_var(&name, &val, true);
                Ok(())
            })?;
        rush_table.set("set_var", set_var_fn)?;

        // rush.get_pwd()
        let get_pwd_fn = self.lua.create_function(|_, ()| {
            let pwd = std::env::current_dir()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            Ok(pwd)
        })?;
        rush_table.set("get_pwd", get_pwd_fn)?;

        // rush.set_alias(name, target)
        let env_alias = Arc::clone(&env);
        let set_alias_fn =
            self.lua
                .create_function(move |_, (name, target): (String, String)| {
                    let mut env_guard = env_alias.lock().unwrap();
                    env_guard.set_alias(&name, &target);
                    Ok(())
                })?;
        rush_table.set("set_alias", set_alias_fn)?;

        // rush.get_alias(name)
        let env_get_alias = Arc::clone(&env);
        let get_alias_fn = self.lua.create_function(move |_, name: String| {
            let env_guard = env_get_alias.lock().unwrap();
            Ok(env_guard.get_alias(&name).map(|s| s.to_string()))
        })?;
        rush_table.set("get_alias", get_alias_fn)?;

        // rush.register_builtin(name, callback)
        let env_builtin = Arc::clone(&env);
        let register_fn =
            self.lua
                .create_function(move |_lua_ctx, (name, _func): (String, LuaFunction)| {
                    let mut env_guard = env_builtin.lock().unwrap();
                    env_guard.register_custom_builtin(&name, move |_env, _args| Ok(0));
                    Ok(())
                })?;
        rush_table.set("register_builtin", register_fn)?;

        // rush.set_prompt(callback)
        let set_prompt_fn = self.lua.create_function(|lua, func: LuaFunction| {
            lua.set_named_registry_value("rush_prompt_fn", func)?;
            Ok(())
        })?;
        rush_table.set("set_prompt", set_prompt_fn)?;

        // rush.set_right_prompt(callback)
        let set_right_prompt_fn = self.lua.create_function(|lua, func: LuaFunction| {
            lua.set_named_registry_value("rush_right_prompt_fn", func)?;
            Ok(())
        })?;
        rush_table.set("set_right_prompt", set_right_prompt_fn)?;

        globals.set("rush", rush_table)?;
        Ok(())
    }

    pub fn eval_script(&self, script: &str) -> Result<()> {
        self.lua.load(script).exec().map_err(ShellError::Lua)
    }

    pub fn load_file(&self, path: &str) -> Result<()> {
        let content = std::fs::read_to_string(path)?;
        self.eval_script(&content)
    }

    pub fn render_prompt(&self) -> Option<String> {
        if let Ok(func) = self
            .lua
            .named_registry_value::<LuaFunction>("rush_prompt_fn")
        {
            func.call::<String>(()).ok()
        } else {
            None
        }
    }

    pub fn render_right_prompt(&self) -> Option<String> {
        if let Ok(func) = self
            .lua
            .named_registry_value::<LuaFunction>("rush_right_prompt_fn")
        {
            func.call::<String>(()).ok()
        } else {
            None
        }
    }
}
