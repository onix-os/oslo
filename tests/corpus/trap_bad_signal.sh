# mode: posix
trap 'echo x' NOSUCHSIGNAL
echo "status=$?"
echo still_running
