# mode: bash
# A shell with no job control has nothing to resume it, so stopping would wedge the session.
suspend 2>/dev/null; echo "s=$?"
suspend -z 2>/dev/null; echo "z=$?"
