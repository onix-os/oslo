# mode: posix
# -r must ask the kernel, not just stat the file.
if [ "$(id -u)" = 0 ]; then echo skipped_root; else
    touch f
    chmod 000 f
    [ -r f ] && echo readable || echo not_readable
    chmod 644 f
fi
