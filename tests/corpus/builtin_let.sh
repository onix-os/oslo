# mode: bash
let x=1+2
echo "$x"
let "y = x * 3"
echo "$y"
let "0"
echo "$?"
let "5"
echo "$?"
if let "x > 2"; then echo big; fi
