# mode: posix
# needs-bash: 5.3
# The width itself moved: bash 5.2 pads the state to 24, 5.3 to 27. oslo follows 5.3, so this
# case is only meaningful against 5.3 or newer.
#
# R7.3: the `jobs` line is bash's, column for column. The state is padded to 27 so the command
# starts in column 34, and a job still running detached keeps the `&` the user typed — oslo used
# to pad to 24 and drop the `&`, which is three columns and two characters of drift from every
# other shell. Only the Running state is asserted: `Done` is spelled differently by `bash --posix`
# (`Done(5)`) and by default bash (`Exit 5`).
sleep 1 &
jobs
echo "status=$?"
kill %1 2>/dev/null
wait 2>/dev/null
