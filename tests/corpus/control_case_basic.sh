# mode: posix
for v in apple banana cherry; do
    case "$v" in
        apple) echo fruit_a ;;
        banana|cherry) echo fruit_bc ;;
        *) echo other ;;
    esac
done
