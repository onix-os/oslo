#!/usr/bin/env bash
# Record one asciinema cast of oslo, driven by a demo script.
#
# The recording is headless — asciinema makes its own pty, tmux renders oslo into it, and this
# script types into the session from outside. Nothing needs a real terminal, so it runs in CI or
# over ssh exactly as it runs here.
#
#   ./scripts/demo/record.sh scripts/demo/nav.demo [out.cast]
#
# The demo script is a line-per-action file:
#
#   cols 120          terminal width  (default 120)
#   rows 20           terminal height (default 20)
#   speed 0.08        seconds between keystrokes while typing
#   run  <text>       type it, then Enter
#   type <text>       type it, leave the line alone
#   key  <name>       one key, as tmux spells it: Right, Enter, F4, C-r, Escape
#   wait <seconds>    pause, so a viewer can read what just happened
#   clear             clear the screen without recording the command
#   #  …              a comment
#
# Every demo runs against the invoking user's real config, so the prompt in the recording is the
# one they actually use, and under OSLO_PROFILE=demo so nothing lands in their history.
set -euo pipefail

DEMO="${1:?usage: record.sh <demo-file> [out.cast]}"
OUT="${2:-/tmp/oslo-demos/$(basename "${DEMO%.demo}").cast}"
OSLO="${OSLO_BIN:-$PWD/target/x86_64-unknown-linux-musl/release/oslo}"
WORK="${DEMO_WORK:-/tmp/oslo-demo-work}"
SESSION="demo_$(basename "${DEMO%.demo}")_$$"

[ -x "$OSLO" ] || { echo "no oslo binary at $OSLO — run: make build" >&2; exit 1; }
command -v asciinema >/dev/null || { echo "asciinema is not installed" >&2; exit 1; }

cols=120 rows=20 speed=0.08
while read -r verb rest; do
    case "$verb" in
        cols) cols="$rest" ;;
        rows) rows="$rest" ;;
        speed) speed="$rest" ;;
    esac
done < "$DEMO"

mkdir -p "$(dirname "$OUT")"
rm -f "$OUT"

tmux kill-session -t "$SESSION" 2>/dev/null || true
tmux -f /dev/null new-session -d -s "$SESSION" -x "$cols" -y "$rows" -c "$WORK" \
    "OSLO_PROFILE=demo $OSLO"
# The status bar would be baked into every frame of the recording; the escape-time default makes
# Escape-then-key look like Alt-key to the shell, which matters for any demo that presses Escape.
tmux set -t "$SESSION" status off
tmux set -t "$SESSION" escape-time 0
sleep 3
tmux send-keys -t "$SESSION" "clear" Enter
sleep 1

asciinema rec --headless --window-size "${cols}x${rows}" \
    --command "tmux attach-session -t $SESSION" "$OUT" >/dev/null 2>&1 &
recorder=$!
sleep 2

send_text() {
    local text="$1" i ch
    for ((i = 0; i < ${#text}; i++)); do
        ch="${text:$i:1}"
        # A lone `;` is tmux's own command separator and never reaches the pane, so `a; b` recorded
        # as `a b` — a demo that silently showed something other than what it said. Sent by hex
        # instead, which the parser does not touch.
        if [ "$ch" = ";" ]; then
            tmux send-keys -t "$SESSION" -H 3b
        else
            tmux send-keys -t "$SESSION" -l -- "$ch"
        fi
        sleep "$speed"
    done
}

while IFS= read -r line; do
    verb="${line%% *}"
    rest="${line#* }"
    [ "$verb" = "$line" ] && rest=""
    case "$verb" in
        ''|'#'*|cols|rows|speed) ;;
        run)   send_text "$rest"; sleep 0.4; tmux send-keys -t "$SESSION" Enter; sleep 1.2 ;;
        type)  send_text "$rest" ;;
        key)   tmux send-keys -t "$SESSION" "$rest"; sleep 0.5 ;;
        wait)  sleep "$rest" ;;
        clear) tmux send-keys -t "$SESSION" "clear" Enter; sleep 0.8 ;;
        *)     echo "unknown verb '$verb' in $DEMO" >&2; exit 1 ;;
    esac
done < "$DEMO"

# **Clear the line before ending it.** A demo that finishes with text still on the prompt — a
# half-typed word, an unaccepted suggestion — would have `exit` typed onto the end of it, and the
# shell would sit there waiting for a command nobody meant, with the recorder waiting on the shell.
tmux send-keys -t "$SESSION" C-c
sleep 0.5
tmux send-keys -t "$SESSION" "exit" Enter
sleep 2

# And if it hangs anyway, end the session rather than the whole run: one stuck demo must not stop
# the other twenty-two from being recorded.
for _ in $(seq 20); do
    kill -0 "$recorder" 2>/dev/null || break
    sleep 0.5
done
if kill -0 "$recorder" 2>/dev/null; then
    echo "  WARNING: $(basename "$DEMO") did not end on its own — killing the session" >&2
    tmux kill-session -t "$SESSION" 2>/dev/null || true
    sleep 1
    kill "$recorder" 2>/dev/null || true
fi
wait "$recorder" 2>/dev/null || true
tmux kill-session -t "$SESSION" 2>/dev/null || true

events=$(grep -c '^\[' "$OUT" || true)
secs=$(grep -oE '^\[[0-9]+\.[0-9]+' "$OUT" | tr -d '[' | awk '{s+=$1} END {printf "%.1f", s}')
printf '%s  %ss  %s events  %s bytes\n' "$OUT" "$secs" "$events" "$(stat -c%s "$OUT")"

# Two ways a recording is quietly ruined, both worth failing on rather than publishing. The status
# bar is baked into every frame; `getcwd` errors mean the working directory was rebuilt underneath a
# running recording, which is what happens when two of these run at once against one fixture.
for wrong in 'getcwd' '\[demo' '0:oslo'; do
    if grep -q "$wrong" "$OUT"; then
        echo "  WARNING: '$wrong' appears in the recording — re-record it" >&2
    fi
done
