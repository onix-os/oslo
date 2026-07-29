# mode: bash
# Setting, then reading back through the same builtin. Every limit here is lowered, never
# raised, so the case does not depend on the privileges of whoever runs it.
ulimit -c 0
ulimit -c

# A soft `unlimited` means "as high as allowed", not "past the hard ceiling".
ulimit -Sc unlimited 2>/dev/null; echo "raise=$?"
ulimit -c

ulimit -n 128
ulimit -n
ulimit -Hn

# File sizes are counted in 512-byte blocks, so this round-trips through the unit conversion.
ulimit -f 4096
ulimit -f

ulimit -f abc 2>/dev/null; echo "nan=$?"
ulimit -Z 2>/dev/null; echo "flag=$?"
