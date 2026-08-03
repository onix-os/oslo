# mode: bash
# `kill` takes a job spec, not only a pid.
#
# This was unimplemented, and the way it failed is the interesting part: `kill %1` wrote to stderr
# and exited 1, so a script that backgrounded something and then signalled it went on to `wait` for
# a job nothing had killed. One corpus script sat there for a full second where bash took two
# milliseconds, and because that script redirected stderr and then blocked, nothing about the
# symptom pointed at `kill`.
#
# Every spelling resolves through the same job table `wait` and `fg` use, so a spec cannot mean one
# thing to one builtin and something else to another.
#
# **`%1` appears once, at the top, and every later case uses `%%`.** That is not style. bash clears
# finished jobs from the table, so a second `sleep 5 &` after a `wait` is `%1` again; oslo keeps
# numbering upward, so `%1` still names the first job — long dead — and `kill` reports ESRCH. This
# file hung four runs in ten while it assumed bash's recycling.
#
# **That divergence is real and is not about `kill`.** It belongs to the job table, it is recorded
# in PERF_AND_CODE.md, and it is deliberately not tested here: a test that fails for a reason other
# than its title is how a suite stops being believed. `%%` means "the current job" under both
# shells, which is what makes the rest of this file say something about `kill` and nothing else.

sleep 5 &
kill %1
echo "numbered: $?"
wait 2>/dev/null

sleep 5 &
kill %%
echo "current: $?"
wait 2>/dev/null

sleep 5 &
kill %+
echo "plus: $?"
wait 2>/dev/null

# A signal by name, not just the default TERM.
sleep 5 &
kill -TERM %%
echo "named-signal: $?"
wait 2>/dev/null

# `kill -0` asks whether it is still there without signalling it, so the job must survive it.
sleep 5 &
kill -0 %%
echo "probe: $?"
kill %%
wait 2>/dev/null

# A job that does not exist is a diagnostic and a failure status, not a crash.
#
# `# mode: bash` rather than `posix` for this file, and this line is the reason: `bash --posix`
# answers **0** for a job spec that names nothing, where plain bash answers 1. Nothing in POSIX asks
# for that — `kill` is specified to fail on an operand it cannot resolve — so oslo matches bash's
# ordinary behaviour and this case is compared against it. Verified both ways:
#
#     bash t.sh          -> missing: 1
#     bash --posix t.sh  -> missing: 0
kill %9 2>/dev/null
echo "missing: $?"
