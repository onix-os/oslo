# mode: bash
# ~+ is the current directory, ~- the previous one, ~user comes from the password database.
[ "$(echo ~+)" = "$PWD" ] && echo pwd_ok || echo pwd_bad
cd /
[ "$(echo ~-)" = "$OLDPWD" ] && echo oldpwd_ok || echo oldpwd_bad
echo ~+/sub
# Both shells read the same password database, so the answers have to agree.
echo ~root
echo ~nosuchuser-xyz
echo ~-x
echo "~+"
