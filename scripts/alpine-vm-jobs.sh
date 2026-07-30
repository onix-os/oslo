#!/bin/oslo
# Job control, run under oslo with a real controlling terminal.
#
# Separate from the main suite because it needs something the main suite cannot have: a session of
# its own with `/dev/ttyS0` as its controlling terminal. Init is handed `/dev/console`, which the
# kernel will not let anything claim as a ctty, so `/init` starts this through `setsid -c`.
#
# What is testable here and nowhere else is the *terminal* half of job control — which process
# group the terminal considers foreground, and whether the shell puts itself back. A test harness
# that is a child process on a pipe has no answer to either question; it is the reason `fg`, `bg`
# and `tcsetpgrp` had no coverage at all before this file.
#
# Deliberately *not* tested here: the INTR and SUSP characters. Generating them means writing into
# the terminal's input queue, which only the far end of the line can do — so that half is driven
# from the host, by `scripts/alpine-vm.sh --console`, which owns qemu's stdin.
echo "VM-JOBS-BEGIN"
# A watchdog, because every check below can block: `wait` on a job the shell has lost track of,
# `fg` on a job that never gets the terminal back. Without it a hang shows up as qemu being killed
# by the outer `timeout`, with no output flushed and nothing to say which check it was.
(
    sleep 90
    echo "VM-JOBS-WATCHDOG-FIRED: a check blocked; the last 'ok' above names it"
    kill -9 $$ 2>/dev/null
) &
watchdog=$!

fails=0
ok() { echo "  ok    $1"; }
no() { echo "  FAIL  $1"; fails=$((fails + 1)); }
check() { if [ "$2" = "$3" ]; then ok "$1"; else no "$1 (expected [$3], got [$2])"; fi; }
# A test that compares two readings is worthless if both come back empty, and empty is exactly
# what happens when the process is already gone — or when the tool asked has no such field.
present() { if [ -n "$2" ]; then ok "$1"; else no "$1 (read nothing at all)"; fi; }

# Process group, session and foreground-group readings come from `/proc/PID/stat` rather than
# `ps`. busybox's `ps` has neither `-o pgid` nor `-p`, so every such check silently read the empty
# string and compared it against another empty string — passing while proving nothing. proc(5) is
# also the only place `tpgid` is available at all.
#
# `$2` counts from the *state* field, because `comm` can contain spaces and parentheses and the
# only reliable way past it is the last `) `: 1=state 2=ppid 3=pgrp 4=session 5=tty_nr 6=tpgid.
procstat() (
    read -r line <"/proc/$1/stat" 2>/dev/null || exit 0
    set -- "$2" ${line#*") "}
    n=$1
    shift
    eval "printf '%s' \"\${$n}\""
)
pgrp_of() { procstat "$1" 3; }
session_of() { procstat "$1" 4; }
tpgid_of() { procstat "$1" 6; }
state_of() { procstat "$1" 1; }

echo "-- the terminal"
tty_name=$(tty 2>/dev/null)
check "stdin is a terminal" "$([ -t 0 ] && echo yes || echo no)" "yes"
present "tty(1) names it" "$tty_name"
# The session leader owns the terminal; `setsid -c` is what arranged that.
sid=$(session_of $$)
present "the shell has a session id" "$sid"

echo "-- process groups under set -m"
set -m
sleep 30 &
bgpid=$!
bgpgid=$(pgrp_of "$bgpid")
shpgid=$(pgrp_of $$)
present "the background job has a pgid" "$bgpgid"
present "the shell has a pgid" "$shpgid"
if [ -n "$bgpgid" ] && [ "$bgpgid" != "$shpgid" ]; then
    ok "job control puts a background job in its own process group"
else
    no "background job shares the shell's process group ($bgpgid = $shpgid)"
fi

echo "-- jobs, and bg on a stopped job"
# `jobs` must know about it, and by the same number `fg`/`bg` accept.
jobs | grep -q "sleep" && ok "jobs lists the running job" || no "jobs does not list it"
kill -STOP "$bgpid" 2>/dev/null
sleep 1
check "the job really stopped" "$(state_of "$bgpid")" "T"
# The shell notices a stop only at a job-table update, which `jobs` performs.
jobs | grep -qi "stopped" && ok "jobs reports a stopped job" || no "jobs does not report the stop"
bg %1 >/dev/null 2>&1
sleep 1
state=$(state_of "$bgpid")
present "the job still exists after bg" "$state"
if [ "$state" = "S" ] || [ "$state" = "R" ]; then
    ok "bg resumed it"
else
    no "bg left it in state [$state]"
fi
kill -TERM "$bgpid" 2>/dev/null
wait "$bgpid" 2>/dev/null
check "a TERMed job reports 128+SIGTERM" "$?" "143"

echo "-- the terminal's foreground process group"
# `tpgid` is what `tcsetpgrp` sets and what the tty driver signals on Ctrl-C. While the shell is
# between commands it must be the shell's own group, or the next Ctrl-C would go nowhere.
tpgid=$(tpgid_of $$)
present "the terminal has a foreground group" "$tpgid"
check "the shell is the foreground group when idle" "$tpgid" "$shpgid"

# ...and during a foreground job the terminal must belong to *that job's* group, not the shell's.
# The child has to read both for us: by the time the shell can look, the job has already finished
# and the shell has taken the terminal back.
fg_report=$(sh -c 'echo "$(sed -n "s/.*) //p" /proc/self/stat | cut -d" " -f3) $(sed -n "s/.*) //p" /proc/self/stat | cut -d" " -f6)' 2>/dev/null)
fg_pgid=${fg_report% *}
fg_tpgid=${fg_report#* }
present "a foreground child reports its pgid" "$fg_pgid"
if [ -n "$fg_pgid" ] && [ "$fg_pgid" != "$shpgid" ]; then
    ok "a foreground job gets its own process group"
else
    no "a foreground job shares the shell's group ($fg_pgid = $shpgid)"
fi
check "the terminal belongs to the foreground job" "$fg_tpgid" "$fg_pgid"

echo "-- signals to the foreground group"
# This is what the tty driver does on Ctrl-C: signal the whole foreground process group. Doing it
# with `kill` tests everything except the driver — that the job dies, that the shell survives, and
# that the status is the one a script tests for.
sh -c 'kill -INT -$$; sleep 5' 2>/dev/null
check "a group SIGINT gives 128+SIGINT" "$?" "130"
check "the shell survived it" "alive" "alive"

sh -c 'kill -QUIT -$$; sleep 5' 2>/dev/null
check "a group SIGQUIT gives 128+SIGQUIT" "$?" "131"

echo "-- the shell keeps the terminal afterwards"
after=$(tpgid_of $$)
present "the terminal still has a foreground group" "$after"
check "the shell took the terminal back" "$after" "$shpgid"
check "the terminal is still usable" "$(echo still-here)" "still-here"

echo "-- wait and status reporting"
sleep 0.2 &
wait $!
check "wait on a clean job is 0" "$?" "0"
(exit 7) &
wait $!
check "wait reports a job's own status" "$?" "7"

kill "$watchdog" 2>/dev/null

echo
if [ "$fails" -eq 0 ]; then echo "JOBS ALL PASSED"; else echo "JOBS $fails FAILED"; fi
exit "$fails"
