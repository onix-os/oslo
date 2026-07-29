# mode: posix
# Quoting inside a pattern is not decoration: it decides, per character, whether a metacharacter
# is one. Every line below answered "yes" before the pattern matcher was given the quoting —
# including `case $x in "$expected")`, which is an ordinary way to compare two strings.
case abc in "*") echo bad-quoted-star ;; *) echo ok-quoted-star ;; esac
case "*" in "*") echo ok-star-matches-itself ;; *) echo bad ;; esac
case abc in \*) echo bad-escaped-star ;; *) echo ok-escaped-star ;; esac
case abc in "a?c") echo bad-quoted-question ;; *) echo ok-quoted-question ;; esac
case "a?c" in "a?c") echo ok-question-literal ;; *) echo bad ;; esac

p="*"
case abc in "$p") echo bad-quoted-var ;; *) echo ok-quoted-var ;; esac
case abc in $p) echo ok-unquoted-var ;; *) echo bad-unquoted-var ;; esac

# Part quoted, part not: each part keeps its own answer.
case "a*z" in "a*"?) echo ok-mixed ;; *) echo bad-mixed ;; esac
case "abz" in "a*"?) echo bad-mixed2 ;; *) echo ok-mixed2 ;; esac

# A `]` that arrived quoted is a member of the bracket expression, not its terminator.
t='ab]cd'
case c in *["${t}"]*) echo ok-bracket-close ;; *) echo bad-bracket-close ;; esac
case e in *[!"${t}"]*) echo ok-bracket-negated ;; *) echo bad-bracket-negated ;; esac
case "]" in [$t]) echo ok-bracket-member ;; *) echo bad-bracket-member ;; esac

# The `${v#p}` family compiles the same patterns.
v="a*c"; echo "hash-quoted=[${v#"a*"}]"
v="axc"; echo "hash-quoted-nomatch=[${v#"a*"}]"
v="axc"; echo "hash-plain=[${v#a*}]"
v="a.b.c"; echo "percent=[${v%%.*}]"

# Unquoted, the `case` subject is not split or globbed either.
s="a b"
case $s in "a b") echo ok-subject ;; *) echo bad-subject ;; esac
