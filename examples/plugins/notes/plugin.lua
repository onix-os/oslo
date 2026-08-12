-- What this plugin declares. Read before it is trusted, in an interpreter with no `oslo` in it —
-- so a manifest can be inspected without being run.
return {
  name     = "notes",
  version  = "0.1.0",
  entry    = "init.lua",
  builtins = { "note" },
  tools    = { "notes" },
}
