# mode: bash
# `;;&` keeps testing the remaining patterns instead of leaving the case.
classify() {
    case $1 in
        [0-9]*) echo "starts with a digit" ;;&
        *[0-9]) echo "ends with a digit" ;;&
        ???) echo "three characters" ;;
        *) echo "no other rule" ;;
    esac
}
classify 1a2
classify abc
classify 42
echo "status=$?"
