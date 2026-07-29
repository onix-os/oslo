# mode: bash
printf 'hello world\nsecond line\n' > in.txt
{ read -n 5 a; echo "n5=[$a]"; read rest; echo "rest=[$rest]"; } < in.txt
{ read -N 4 b; echo "N4=[$b]"; } < in.txt
{ read -d ' ' c; echo "d=[$c]"; } < in.txt
{ read -r -n 3 d; echo "rn=[$d]"; } < in.txt
{ read -rn3 e; echo "cluster=[$e]"; } < in.txt
{ read -p 'PROMPT> ' f; echo "p=[$f]"; } < in.txt
{ read -s g; echo "s=[$g]"; } < in.txt
{ read -u 0 h; echo "u=[$h]"; } < in.txt
{ read -t 0 i; echo "t0=$?"; } < in.txt
{ read -- j; echo "dd=[$j]"; } < in.txt
printf 'a:b:c\n' > f.txt
{ IFS=: read -a arr; echo "arr=[$arr]"; } < f.txt
