# mode: bash
# ${!name} expands the parameter that name names.
target=payload
name=target
echo "${!name}"
# Only the inner lookup may come up empty: naming an unset parameter is fine.
name=nosuchvar
echo "[${!name}]"
# The name can be a positional, and the indirection re-reads it each time.
set -- first second
p=1
echo "${!p}"
p=2
echo "${!p}"
