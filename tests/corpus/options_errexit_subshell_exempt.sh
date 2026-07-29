# mode: posix
# The exemption crosses the fork: a subshell in a condition runs to its end.
set -e
(false) || echo rescued
if (false; echo sub_continued); then echo cond_true; fi
echo survived
