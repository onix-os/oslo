# mode: bash
# oslo refuses coproc by name rather than running its body inline; bash runs it for real.
coproc counter { echo one; }
echo "started=$?"
