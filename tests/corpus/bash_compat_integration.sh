# mode: bash
# The four shell-level things a bash integration needs before its keybindings can exist.
#
# Each of these was found by loading atuin under oslo and watching where it stopped. None of them
# is exotic; all four are ordinary bash that oslo got wrong, and each one on its own was enough to
# make the integration install cleanly and then do nothing.

# 1. An array subscript inside arithmetic.
#
# `((BASH_VERSINFO[0] < 3))` is atuin's include guard. oslo's arithmetic tokeniser had no `[`, so
# the guard was a hard error and the whole integration was skipped.
versions=(4 2 0)
echo "subscript: $((versions[0]))"
echo "expression subscript: $((versions[1 + 1]))"
i=1
echo "variable subscript: $((versions[i]))"
((versions[2] = 9))
echo "assign through subscript: ${versions[2]}"
((versions[2]++))
echo "step through subscript: ${versions[2]}"
echo "missing element is zero: $((versions[7]))"

# 2. A declaration builtin's operands are assignments, not words.
#
# `local command=${2/#"$widget"/__atuin_history --keymap-mode=emacs}` is the line that broke.
# Field-splitting the value meant `local` saw two arguments and bound everything before the first
# space — so every keybinding atuin computed was silently truncated to its first word.
show() {
    local joined=$(echo one two three)
    echo "local keeps spaces: [$joined]"
    local from_default=${unset_name:-a b c}
    echo "and in an expansion: [$from_default]"
}
show

declare declared=$(echo four five)
echo "declare too: [$declared]"

export exported=$(echo six seven)
echo "export too: [$exported]"

# A value that would glob must not: an assignment is not a word.
mkdir -p subscripts && touch subscripts/one subscripts/two
globbed() {
    local pattern=subscripts/*
    echo "no globbing: [$pattern]"
}
globbed
rm -rf subscripts

# But a *non*-assignment operand still splits, or `local $spec` stops working.
spec="split_me=value"
splitting() {
    local $spec
    echo "a bare operand still expands normally: [$split_me]"
}
splitting

# 3. `eval --`.
#
# atuin dispatches every widget through `builtin eval -- "$widget"`. Without `--` being consumed
# oslo looked for a command named `--`, so each keypress answered "command not found" and the
# search never opened.
eval -- "echo eval-with-dashes"
builtin eval -- "echo builtin-eval-with-dashes"
eval -- echo joined args
eval --
echo "empty eval: $?"

# 4. `$BASH_VERSION` and `$BASH_VERSINFO` exist.
#
# Not a claim to be bash — a declaration of which bash an integration may assume. Absent, they read
# as version 0, which is older than any bash that ever shipped, and every guard of the form
# `((BASH_VERSINFO[0] < 3))` concluded the shell was from before 1996.
#
# The values are deliberately not asserted here: they are oslo's to choose and will rise as
# features land. What has to hold is that they are present, numeric, and consistent with each
# other, because that is all a guard actually tests.
if [ -n "$BASH_VERSION" ]; then echo "version string: present"; fi
if [ ${#BASH_VERSINFO[@]} -ge 5 ]; then echo "versinfo: at least five fields"; fi
case "$BASH_VERSION" in
    "${BASH_VERSINFO[0]}.${BASH_VERSINFO[1]}."*) echo "they agree" ;;
    *) echo "they disagree: $BASH_VERSION vs ${BASH_VERSINFO[0]}.${BASH_VERSINFO[1]}" ;;
esac
if ((BASH_VERSINFO[0] >= 3)); then echo "the guard every integration writes: passes"; fi
