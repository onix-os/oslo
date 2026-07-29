# mode: bash
# A parameter whose value is not a name aborts the expansion — it must not expand to empty.
echo before
n="not a name"
echo "[${!n}]"
