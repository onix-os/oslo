-- oslo.nix, the named half — written in Lua so that it can be replaced.
--
-- Everything here is built on `oslo.nix.run`, which is the only part of this feature that is Rust.
-- A config or a plugin overwrites any of these by assigning to `oslo.nix.<name>`, and adds its own
-- the same way; nothing below is privileged.
--
-- Every function takes one optional table of options:
--
--   flake    an installable to ask about, instead of the current directory
--   cache    keep the answer until the flake changes  (default: ask nix, which caches too)
--   timeout  seconds before nix is killed            (default: 60)

return function(nix)
  -- `oslo.json` tags every decoded table with `__json` so a document keeps its shape when it is
  -- encoded again. That tag is in `pairs` like any other key, so anything walking a decoded object
  -- has to step over it — without this, every flake would report an input named `__json`.
  local TAG = "__json"

  local function keys(t)
    local out = {}
    if type(t) ~= "table" then return out end
    for k in pairs(t) do
      if k ~= TAG then out[#out + 1] = k end
    end
    table.sort(out)
    return out
  end

  -- The one call the rest go through: nix's arguments, plus the caller's options.
  local function ask(argv, opts)
    opts = opts or {}
    if opts.flake then argv[#argv + 1] = opts.flake end
    argv.cache = opts.cache
    argv.timeout = opts.timeout
    return nix.run(argv)
  end

  --- The flake's own description, revision, inputs and store path.
  function nix.metadata(opts)
    return ask({ "flake", "metadata" }, opts)
  end

  --- Every output the flake defines, by system.
  function nix.outputs(opts)
    return ask({ "flake", "show" }, opts)
  end

  --- nix's settings, each as `{ value = …, description = … }`.
  function nix.config(opts)
    return ask({ "config", "show" }, opts)
  end

  --- The system nix builds for here — `x86_64-linux`.
  function nix.system(opts)
    local config, err = nix.config(opts)
    if not config then return nil, err end
    local setting = config.system
    return type(setting) == "table" and setting.value or nil
  end

  --- Does the flake have uncommitted changes?
  ---
  --- A different question from `git status`: a flake only ever sees files git is tracking, so a
  --- new-but-unadded file leaves this false while git calls the tree dirty.
  function nix.dirty(opts)
    local metadata, err = nix.metadata(opts)
    if not metadata then return nil, err end
    return type(metadata.dirtyRevision) == "string"
  end

  --- What every input is pinned to, oldest first.
  ---
  --- Each entry is `{ name, type, pinned, days }`, where `pinned` is a unix time and `days` is how
  --- long ago that was. This is the whole reason to read the lock: it costs no evaluation — 27 ms
  --- against a warm nix — and nothing else in the shell can tell you a project is pinned to a
  --- three-year-old `nixpkgs`.
  function nix.inputs(opts)
    local metadata, err = nix.metadata(opts)
    if not metadata then return nil, err end
    local nodes = type(metadata.locks) == "table" and metadata.locks.nodes
    if type(nodes) ~= "table" then return {} end

    local now, found = os.time(), {}
    for _, name in ipairs(keys(nodes)) do
      -- The root node is the flake itself and has nothing pinned; everything else does.
      local locked = type(nodes[name]) == "table" and nodes[name].locked
      if type(locked) == "table" and type(locked.lastModified) == "number" then
        found[#found + 1] = {
          name = name,
          type = locked.type,
          pinned = locked.lastModified,
          days = math.floor((now - locked.lastModified) / 86400),
        }
      end
    end
    table.sort(found, function(a, b) return a.days > b.days end)
    return found
  end

  --- The names of the dev shells this machine can enter.
  ---
  --- `default` is the one `nix develop` picks with no argument, and it is listed like any other.
  function nix.shells(opts)
    local outputs, err = nix.outputs(opts)
    if not outputs then return nil, err end
    if type(outputs.devShells) ~= "table" then return {} end
    local system = (opts and opts.system) or nix.system(opts)
    if not system then return {} end
    return keys(outputs.devShells[system])
  end

  -- Which outputs each subcommand can be given, in the order they are offered.
  local WANTS = {
    develop = { "devShells" },
    build = { "packages", "checks" },
    run = { "apps", "packages" },
    shell = { "packages" },
    profile = { "packages" },
    bundle = { "packages" },
  }

  --- Complete `nix build .#<TAB>` and its relatives, from the flake's own outputs.
  ---
  --- Not installed by anything: oslo completes nothing for `nix` unless a config says so, which is
  --- one line.
  ---
  ---     oslo.completion.for_command.nix = oslo.nix.complete
  ---
  --- Answers `nil` to fall through to oslo's ordinary completion — files and flags — for every
  --- subcommand and word this has nothing to say about.
  function nix.complete(argv, current)
    local wants = WANTS[argv[2]]
    -- A flag is oslo's business, not this one's.
    if not wants or current:sub(1, 1) == "-" then return nil end

    local ref, stem = current:match("^(.-)#(.*)$")
    if not ref then ref, stem = ".", current end
    -- **Only the flake you are standing in.** `nix flake show nixpkgs` evaluates the whole of
    -- nixpkgs; the same class of query took 46 seconds here when it was `nix search`. Nothing that
    -- runs on a keystroke may risk that, so a named flake falls through to ordinary completion.
    if ref ~= "." and ref ~= "" then return nil end

    -- Cached, because this runs per Tab. Warm, nix answers `flake show` in 34 ms and the cache in
    -- well under one; cold it is 455 ms, which is a wait nobody should pay twice.
    local outputs = nix.outputs { cache = true }
    local system = nix.system { cache = true }
    if not outputs or not system then return nil end

    local found = {}
    for _, kind in ipairs(wants) do
      local by_system = outputs[kind]
      local here = type(by_system) == "table" and by_system[system]
      for _, name in ipairs(keys(here)) do
        if name:sub(1, #stem) == stem then
          local about = type(here[name]) == "table" and here[name].description
          if about == "" then about = nil end
          found[#found + 1] = { ref .. "#" .. name, about }
        end
      end
    end
    return found
  end
end
