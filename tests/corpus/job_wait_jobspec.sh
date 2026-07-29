# mode: posix
# %n and %% name a job the same way `$!` names its pid.
sh -c 'exit 5' &
wait %1
echo "n=$?"
sh -c 'exit 6' &
wait %%
echo "current=$?"
wait %9
echo "missing=$?"
