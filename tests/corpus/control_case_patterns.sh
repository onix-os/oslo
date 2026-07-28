# mode: posix
for v in file.txt file.log dir/ ""; do
    case "$v" in
        *.txt) echo text ;;
        *.log) echo log ;;
        */) echo dir ;;
        "") echo empty ;;
        *) echo unknown ;;
    esac
done
