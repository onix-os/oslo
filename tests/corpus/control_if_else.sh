# mode: posix
if true; then echo then_branch; else echo else_branch; fi
if false; then echo then_branch; else echo else_branch; fi
if false; then echo a; elif true; then echo elif_branch; else echo c; fi
if false; then echo a; elif false; then echo b; else echo else_branch; fi
