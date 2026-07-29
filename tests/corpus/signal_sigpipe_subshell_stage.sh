# mode: posix
# Same requirement one fork deeper: an explicit subshell as the writing end of a pipe.
( while true; do echo x; done ) | head -1
echo "status=$?"
