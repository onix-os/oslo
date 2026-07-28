# mode: bash
# ${v/pat/rep} and ${v//pat/rep}
v=a-b-c
echo "${v/-/+}"
echo "${v//-/+}"
echo "${v/#a/A}"
echo "${v/%c/C}"
p=one.two.three
echo "${p//./ }"
