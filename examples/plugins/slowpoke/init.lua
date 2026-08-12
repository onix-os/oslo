-- A ghost that has to go and ask something — the shape an LLM plugin has.
--
-- **Deliberately slow and deliberately deterministic.** It shells out to `sleep` and then answers
-- from a table, so it exercises every part of the asynchronous path — the debounce, the reply
-- carrying the line it was asked about, the repaint when the answer lands — while still being
-- something a test can assert on. Swap the `oslo.spawn` for a call to a model and it is the real
-- thing; nothing else about this file would change.

local answers = {
  ["git s"] = "git status --short",
  ["docker p"] = "docker ps --all",
}

oslo.suggest.provider {
  name = "slowpoke",

  -- Wait for typing to stop. Ten keystrokes in one word is one question, not ten — which is the
  -- difference between a plugin that costs a fraction of a penny and one that costs ten.
  debounce_ms = 150,
  timeout_ms  = 3000,

  -- Draw only if nothing else did. Change to "replace" to have it beat your history — that is the
  -- decision this setting exists for, and it is yours rather than this plugin's.
  on_late  = "fill",
  settle_ms = 400,

  -- Short lines are not questions worth asking a model.
  min_chars = 4,

  -- The context rule. A predicate is what says *not here* — for a provider that would send your
  -- typing off the machine, this is the setting that matters most.
  enabled = function(ctx)
    return not ctx.cwd:match("/private")
  end,

  request = function(ctx, reply)
    local said = answers[ctx.line]
    if not said then return reply(nil) end
    -- The work happens off the prompt; `reply` is called from the callback, whenever that is.
    oslo.spawn { "sleep", "0.2", on_exit = function() reply(said) end }
  end,
}

oslo.plugin.test("it knows what it is going to say", function(t)
  t.equal(answers["git s"], "git status --short", "the answer table is what it replies with")
  t.ok(answers["git s"]:sub(1, 5) == "git s", "and every answer continues its line")
end)
