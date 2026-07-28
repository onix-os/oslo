# mode: posix
case x in y) echo no ;; esac
echo "$?"
case x in x) false ;; esac
echo "$?"
