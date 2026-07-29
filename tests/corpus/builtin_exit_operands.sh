# mode: posix
# `exit` has to agree with the shell about what its operand means: bare `exit` carries $? out,
# an out-of-range code is folded into the low byte, and a usage error is refused rather than
# turned into success.
(exit)
echo "bare=$?"
(false; exit)
echo "last=$?"
(exit 300)
echo "wrap=$?"
(exit -1)
echo "neg=$?"
(exit 1 2)
echo "many=$?"
