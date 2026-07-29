# mode: bash
declare X=1
echo "$X"
f() { declare Y=inner; echo "$Y"; }
Y=outer
f
echo "$Y"
declare -p X
typeset Z=ksh
echo "$Z"
