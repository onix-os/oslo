# mode: posix
# A pipeline stage running *shell* code is a forked child too, so it needs the same SIG_DFL
# treatment as an exec'd one. With SIGPIPE still ignored the builtin write returns EPIPE, the
# loop never notices its reader is gone, and the shell spins forever instead of dying.
while true; do echo x; done | head -1
echo "status=$?"
