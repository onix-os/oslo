print("--- Lua Script Execution inside rush ---")
rush.set_var("LUA_VAR", "Hello from Lua!")
print("LUA_VAR set to: " .. rush.get_var("LUA_VAR"))
rush.exec("echo 'Executing shell commands from Lua script:'; pwd")
