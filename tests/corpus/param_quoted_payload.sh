# mode: bash
# Inside double quotes the payload of `${x-word}` keeps its own quotes, backslashes and tildes: the
# enclosing context governs, and the payload is not re-lexed as a word of its own. The unquoted
# spellings already agreed, so only the double-quoted uses were wrong — silently.
echo "single=[${x-'q'}]"
echo "backslash=[${x:-a\ b}]"
echo "tilde=[${x:-~}]"
echo "bare_tilde=[${x:-~}]" | cat
echo "unquoted_single=[${x-'q'}]"
echo "dollar_still_expands=[${x:-$HOME}]" | sed "s|$HOME|HOME|"
echo "escaped_dollar=[${x:-\$lit}]"
echo "empty=[${x-}]"
set_one=here
echo "present_wins=[${set_one-'q'}]"

# A pattern is not a payload: it processes its own quotes in either context.
v=abc
echo "prefix=[${v#'a'}] [${v##'a'}]"
echo "suffix=[${v%'c'}] [${v%%'c'}]"
p=/a/b.c
echo "idioms=[${p##*/}] [${p%.*}]"
