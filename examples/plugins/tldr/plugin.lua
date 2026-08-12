-- What this plugin declares. Read before it is trusted, in an interpreter with no `oslo` in it.
return {
  name     = "tldr",
  version  = "0.1.0",
  entry    = "init.lua",
  builtins = { "tldr" },
  load_on  = "pre-prompt",
}
