# mode: bash
# R8.7: a timed pipeline is still an ordinary pipeline — `!`, `&&`, `$?` and `set -e`'s
# exemptions all see through the keyword to the status underneath.
if time true; then echo cond_ran; fi
time false || echo or_ran
time true && echo and_ran
time ! false
echo "negated=$?"
set -e
time false || echo errexit_exempt
set +e
time (exit 4)
echo "subshell=$?"
