# mode: bash
# In bash, brace expansion is a purely textual step that runs before parameter expansion, so an
# alternative ending in an expansion fuses with the text after the group into a single name:
# `{$v,y}z` becomes `$vz` and `yz`, and `$vz` is unset.
v=x
echo {$v,y}z
echo pre{$v,y}post
