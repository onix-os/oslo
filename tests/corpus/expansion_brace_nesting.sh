# mode: bash
# Brace groups nest, multiply with their neighbours, and stay literal when they are not groups.
echo a{b,c}d
echo {a,b}{1,2}
echo {a,b{c,d}}
echo a{,}b
echo a{b,}
echo a{b}c
echo {}
echo {a}{b,c}
echo {a,b
echo }a{
echo {a{b,c}
echo pre{a,b}post
