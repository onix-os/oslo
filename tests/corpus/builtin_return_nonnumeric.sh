# mode: bash
# A bad operand still returns from the function, with status 2 — it does not fall through to the
# rest of the body. Not `# mode: posix`: there `return` is a special builtin whose usage error
# takes the whole shell down, which is a separate question from what the operand means.
f() {
    return abc
    echo after
}
f
echo "rc=$?"
