# mode: bash
# R1.7: an invalid identifier reaches `env::set_var`, which panics. `export "=1"` used to abort
# the process (exit 101) and take an interactive session with it.
#
# The oracle is plain bash on purpose. Under `--posix` a special builtin that fails exits the
# shell, so `bash --posix` never reaches the sentinel — which says something about rush's special-
# builtin handling, not about whether it survived the bad name. That claim gets its own case in
# robust_special_builtin_failure.sh; this one is only about staying alive.
export "=1" 2>/dev/null
echo "status=$?"
echo STILL_ALIVE
