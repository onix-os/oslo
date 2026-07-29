# mode: posix
# A signal spec is resolved before anything is sent, in every spelling.
kill -0 $$
echo "probe=$?"
kill -s 0 $$
echo "dash_s=$?"
kill -n 0 $$
echo "dash_n=$?"
kill -s NOSUCH $$ 2>/dev/null
echo "bad_name=$?"
kill -99 $$ 2>/dev/null
echo "bad_num=$?"
echo STILL_ALIVE
