# mode: posix
# kill -l translates in both directions. The SIG-prefixed spelling is deliberately absent:
# bash --posix rejects `kill -l SIGHUP` and oslo accepts it, and that leniency is not a bug
# worth freezing into the oracle.
kill -l 9
kill -l 15
kill -l KILL
kill -l hup
echo "status=$?"
kill -l NOSUCH 2>/dev/null
echo "bad=$?"
