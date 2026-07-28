# mode: posix
[ abc = abc ]; echo "$?"
[ abc != abc ]; echo "$?"
[ -z "" ]; echo "$?"
[ -n "" ]; echo "$?"
[ "" = "" ]; echo "$?"
