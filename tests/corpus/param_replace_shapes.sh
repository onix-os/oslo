# mode: bash
# Every shape the replacement operators take, because the fast paths added for speed must not have
# changed any answer: a literal pattern is now a substring search, a pattern with no `*` has a fixed
# width and tests one end rather than every end, and only a starred pattern walks them all.
v=abcabc
echo "all=[${v//b/X}] one=[${v/b/X}]"
echo "any=[${v//?/X}] fixed=[${v//a?c/X}] star=[${v//a*/X}]"
echo "class=[${v//[bc]/X}]"
w=aaa
echo "greedy=[${w//aa/X}]"
echo "absent=[${v//zz/X}] empty=[${v///X}]"
echo "amp=[${v//?/[&]}]"
echo "anchored=[${v/#abc/X}] [${v/%abc/X}]"
u=café
# Not `${u//?/X}`: oslo counts characters and bash in the C locale this harness runs under
# counts bytes, so that one compares locales rather than replacements.
echo "utf8=[${u//é/e}] [${u//caf/X}] [${u//é/}]"
p=/a/b.c
echo "idioms=[${p##*/}] [${p%.*}] [${p#/}] [${p%%.*}]"
