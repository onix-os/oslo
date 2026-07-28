# mode: posix
# A non-exported variable must not appear in a child's environment.
secret=classified
export shown=public
env | grep -c '^secret='
env | grep -c '^shown='
