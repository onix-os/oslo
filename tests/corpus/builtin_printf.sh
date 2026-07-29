# mode: posix
# `printf` is built in, because a distro's /bin/sh runs before coreutils is on the filesystem —
# an initramfs, an early boot script, a stage-0 chroot. It is also the only portable way to write
# a string with no trailing newline, `echo -n` not being POSIX.
#
# The two escape passes are the subtle part and are both asserted: the *format* always has its
# backslashes decoded, an *argument* never does unless `%b` consumes it.
printf 'plain\n'
printf '%s|%s\n' a b
printf '%s\n' one two three
printf '[%5d][%-5d][%05d]\n' 42 42 42
printf '[%5s][%-5s][%.2s]\n' ab ab abcdef
printf '%x %X %o %c\n' 255 255 8 hello
printf '%%\n'
printf 'tab\there\n'
printf '%s\n' 'not\tdecoded'
printf '%b\n' 'is\tdecoded'
printf '%.3f\n' 3.14159
printf '%e\n' 1234.5
printf '%d %d %d\n' 0x1f 010 "'A"
printf '[%s][%s]\n' only
printf '%d\n' abc 2>/dev/null; echo "bad-number=$?"
# `z` is a C length modifier, so bash reads `%zb` as `%b` — treating it as the conversion letter
# made a valid format an error.
printf 'a%zb\n' x; echo "modifier=$?"
# `%q` quotes so the shell reads the string back unchanged; bash backslash-escapes rather than
# wrapping in quotes, and the corpus compares bytes.
printf '%q %q %q %q\n' 'a b' "it's" '' plain
printf 'a%wb\n' x 2>/dev/null; echo "bad-format=$?"
