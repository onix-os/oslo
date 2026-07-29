# mode: bash
# PIPESTATUS reports every stage of the last pipeline, left to right.
false | true | false
echo "${PIPESTATUS[@]}"
echo "${#PIPESTATUS[@]}"
echo "${PIPESTATUS[0]} ${PIPESTATUS[2]}"
# A one-command pipeline records a single status.
true
echo "${PIPESTATUS[@]}"
(exit 3)
echo "${PIPESTATUS[@]}"
