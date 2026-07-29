# mode: bash
# R8.7: `time` reports on stderr only — stdout and the pipeline's status are untouched.
# The numbers themselves are not reproducible, so only their absence from stdout is compared.
time echo timed
echo "after=$?"
time false
echo "failed=$?"
captured=$(time echo inside 2>/dev/null)
echo "[$captured]"
time { echo group_a; echo group_b; }
time echo piped | cat
