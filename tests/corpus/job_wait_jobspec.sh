# mode: posix
# %n and %% name a job the same way `$!` names its pid.
#
# Each job sleeps first so that it is still in the table, and still running, when the jobspec is
# resolved. bash cleans dead jobs out of a non-interactive shell's table on its own schedule, so a
# jobspec for a job that has already exited is answered sometimes and refused sometimes: `wait %%`
# gave 6 on one CI run and 127 on the next from the same commit. A pid, which the surrounding
# cases use, keeps its status either way — a jobspec does not, so this case has to name a job that
# is still alive to be asking a question with one answer.
sh -c 'sleep 0.2; exit 5' &
wait %1
echo "n=$?"
sh -c 'sleep 0.2; exit 6' &
wait %%
echo "current=$?"
wait %9
echo "missing=$?"
