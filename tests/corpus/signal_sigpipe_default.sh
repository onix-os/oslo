# mode: posix
# A child must not inherit the SIG_IGN the Rust runtime installs for SIGPIPE: `yes` has to die
# on the closed pipe, quietly, instead of surviving the write and reporting "Broken pipe".
yes | head -1
echo "status=$?"
