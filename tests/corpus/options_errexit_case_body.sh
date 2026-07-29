# mode: posix
set -e
case abc in
  a*) echo matched; false; echo NOT_REACHED ;;
  *) echo NO ;;
esac
echo NOT_REACHED_EITHER
