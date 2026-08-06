# mode: bash
# A leading backslash on the command word, in a script.
#
# oslo gives `\cmd` and `\\cmd` extra meanings at a prompt — `\cmd` reaches past the builtin to the
# program on $PATH, `\\cmd` keeps the alias and skips the builtin. **None of that reaches a
# script**, and this case is what proves it rather than the argument that it should not.
#
# It matters because oslo is meant to be /bin/sh: every `\ls` and `\echo` already written on the
# machine has to go on meaning what POSIX says it means, and a change there would be silent.

# The POSIX job of a backslash and the only one: the alias does not expand.
shopt -s expand_aliases
alias greet='echo aliased'
greet
\greet 2>/dev/null
echo "escaped alias status=$?"

# And then ordinary command search resumes — so a *function* by that name still answers, which is
# exactly what the interactive reading of `\cmd` would have skipped.
probe() { echo FUNCTION; }
probe
\probe
echo "function status=$?"

# The builtin too. `\echo` is `echo`, and `echo` is a builtin in both shells.
\echo builtin reached

# `\\cmd` is a literal backslash followed by a word: a command whose name starts with `\`, which
# nothing has. There is no POSIX meaning here to protect, which is why the interactive form is free
# to give it one.
\\probe 2>/dev/null
echo "double status=$?"

# An escape anywhere but the front is nothing special — `pr\obe` is `probe`.
pr\obe

# Quoting is not escaping. Both forms run the builtin, which is what dispatch tables written as
# "$cmd" depend on.
"echo" quoted double
'echo' quoted single
