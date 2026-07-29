# mode: bash
# In bash, brace expansion is a purely textual step that runs before the word is lexed, so a group
# boundary lands *inside* whatever token is adjacent to it. Both edges fuse: `{$v,y}z` becomes
# `$vz` and `yz`, and `$v{a,b}` becomes `$va` and `$vb`. Every one of those names a variable that
# does not exist, and only `${v}` closes the name against the group.
v=x
echo {$v,y}z
echo pre{$v,y}post
echo $v{a,b}
echo ${v}{a,b}
echo ${v}z
# It is the *name* that grows, and a positional parameter's name is one digit long.
set -- A B
echo {$1,y}0
# An expansion is opaque to the pass, comma and all.
echo {${x:-a,b},y}
echo {$(echo p,q),y}
