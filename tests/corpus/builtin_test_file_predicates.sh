# mode: posix
touch regular
mkdir directory
[ -f regular ] && echo is_file
[ -d directory ] && echo is_dir
[ -e regular ] && echo exists
[ -e missing ] || echo no_missing
[ -s regular ] || echo empty_file
echo content > nonempty
[ -s nonempty ] && echo has_content
[ -r regular ] && echo readable
[ -x regular ] || echo not_executable
