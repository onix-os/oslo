# mode: posix
# The exemption is dynamic: it covers whatever the exempt command reaches, functions included.
set -e
f() {
  false
  echo f_continued
}
f || echo rescued
if f; then echo cond_true; fi
! f
while f; do break; done
echo survived
