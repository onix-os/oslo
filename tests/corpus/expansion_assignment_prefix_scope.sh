# mode: posix
# A prefix assignment keeps its RHS whole, exports it, and does not outlive the command.
IFS=:
inner=$(v=a:b:c sh -c 'echo "$v"')
echo "$inner"
echo "[$v]"
outer=p:q
seen=$(outer=r:s sh -c 'echo "$outer"')
echo "$seen"
echo "$outer"
