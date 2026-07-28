# mode: posix
# A pattern that does not match leaves the value alone.
v=hello
echo "${v#xyz}"
echo "${v%xyz}"
echo "${v#h}"
echo "${v%o}"
