# mode: bash
# oslo names `select` as unsupported instead of dying at the `in`; bash runs the loop, reads EOF
# from /dev/null and falls out of it.
select choice in one two; do
    echo "picked=$choice"
done
echo "after=$?"
