# mode: posix
# Twenty-five parentheses that never close. brush_parser is a PEG, so an unmatched opener makes
# it backtrack exponentially: rush used to sit at 100% CPU and never return, while bash reports
# the syntax error immediately. parser::nesting counts what is still open and refuses first.
(((((((((((((((((((((((((x
echo "never reached=$?"
