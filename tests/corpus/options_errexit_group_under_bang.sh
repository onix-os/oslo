# mode: posix
# `!` exempts the whole pipeline, so a brace group under it runs past its first failure.
set -e
! { false; echo group_continued; }
echo after_bang_group
until ! false; do echo NO; done
echo survived
