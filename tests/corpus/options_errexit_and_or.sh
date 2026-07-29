# mode: posix
# Every command of an and-or list but the last is exempt, so a short-circuited chain is harmless.
set -e
false || echo rescued
false && echo NOT_RUN
echo after_short_circuit
false || false || echo third
true && true && echo chained
echo survived
