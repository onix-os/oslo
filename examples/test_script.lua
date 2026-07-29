print("--- Lua Script Execution inside oslo ---")
oslo.set_var("LUA_VAR", "Hello from Lua!")
print("LUA_VAR set to: " .. oslo.get_var("LUA_VAR"))
oslo.exec("echo 'Executing shell commands from Lua script:'; pwd")
