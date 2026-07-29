# mode: bash
# {n..m} counts either way, takes a step magnitude, and pads when an operand has a leading zero.
echo {1..4}
echo {4..1}
echo {-2..2}
echo {0..10..3}
echo {1..5..-2}
echo {08..11}
echo {-01..1}
echo x{1..3}y
echo {1..3}{a,b}
# Letters are a sequence too; a malformed one is just text.
echo {a..e}
echo {e..a..2}
echo {1..z}
echo {1...5}
echo {a..b..c}
echo {1..3..0}
