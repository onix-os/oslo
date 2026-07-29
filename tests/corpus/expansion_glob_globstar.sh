# mode: posix
# POSIX has no globstar: ** is an ordinary * and still cannot cross a /.
mkdir d
mkdir d/deep
touch d/x1
touch d/deep/x2
touch top1
echo **
echo **/*
echo d/**
