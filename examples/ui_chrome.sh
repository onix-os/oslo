# Every widget option that is about *presentation* rather than about the question.
#
#   oslo examples/ui_chrome.sh
#
# Four options, and they compose. `examples/ui_tour.sh` shows the widgets themselves; this one
# shows what can be wrapped around any of them:
#
#   --no-legend                    hide the `↑↓ move • enter choose` row
#   --border B [--border-fg C]     rounded / square / double / thick
#   --border-fit content|full      hug the content, or reach the edges of the terminal
#   --fullscreen                   draw on the alternate screen, out of the scrollback
#   --align-x / --align-y / --align   start|left|top, center, end|right|bottom
#
# The same four are Lua table fields — `legend`, `border`, `fit`, `fullscreen`, `align_x`,
# `align_y` — on every `oslo.ui.*` widget. One parser builds the same `Chrome` for both, so a
# prompt looks identical whether shell or Lua asked for it.
#
# Every step reports its exit status, because that status *is* the interface: 0 answered,
# 1 cancelled, 2 nobody to ask. Esc cancels any widget.

set -u

step=0
say() {
    step=$((step + 1))
    printf '\n'
    ui style --border rounded --padding "0 1" --border-fg 5 "$step/10  $1"
}
note() { ui log --level info "$1"; }
pause() {
    printf '\n'
    ui confirm --yes next --no stop "carry on?" || {
        note "stopped at step $step"
        exit 0
    }
}

say "the default: a legend, no border, top left"
ui choose --header "pick one" alpha beta gamma
note "status $?"
pause

# --------------------------------------------------------------------- legend

say "--no-legend: the same list without the key row"
# For a widget inside something that already explains itself — a wizard whose header says what the
# keys are, or a list so short the keys are obvious. The two rows go back to the list rather than
# leaving a gap where they were.
ui choose --no-legend --header "no key row" alpha beta gamma
note "status $?"
pause

# --------------------------------------------------------------------- border

say "--border rounded: hugging the content"
ui choose --border rounded --border-fg 4 --header "in a box" alpha beta gamma
note "status $?"
pause

say "--border double --border-fit full: reaching the edges"
# Content-width is right for a prompt sitting in a transcript. Full-width is right when the widget
# is the only thing on screen, and is the one that looks deliberate rather than stranded.
ui choose --border double --border-fg 2 --border-fit full --header "full width" alpha beta gamma
note "status $?"
pause

say "every border, so you can pick one"
for b in rounded square double thick; do
    ui style --border "$b" --padding "0 1" --border-fg 6 "$b"
done
pause

# ----------------------------------------------------------------- fullscreen

say "--fullscreen: the alternate screen"
# The terminal keeps a second buffer that is not part of the scrollback. Leaving it puts the
# transcript back exactly — which is why this is better than clearing the screen, and why the
# widget restores it even if you cancel.
ui choose --fullscreen --header "a screen of its own — esc to come back" alpha beta gamma delta
note "status $?  (and the transcript is back)"
pause

# ------------------------------------------------------------------ alignment

say "--fullscreen --align center: in the middle of it"
ui choose --fullscreen --align center --border rounded --border-fg 5 \
    --header "centred both ways" alpha beta gamma
note "status $?"
pause

say "--align-x center on its own: horizontally centred, still inline"
# Horizontal placement works inline. Vertical does not, deliberately: pushing a widget down the
# screen when it does not own the screen would scroll away the transcript above it.
ui choose --align-x center --border rounded --header "centred across" alpha beta gamma
note "status $?"
pause

say "--fullscreen --align-x center --align-y bottom"
ui choose --fullscreen --align-x center --align-y bottom --border thick --border-fg 3 \
    --header "bottom middle" alpha beta gamma
note "status $?"
pause

# ------------------------------------------------------------- other widgets

say "the same four on the other widgets"
# Not a property of `choose`: every widget that draws a frame takes them.
ui input --border rounded --border-fg 4 --prompt "input in a box: " --placeholder "type here"
note "input status $?"

ui confirm --border square --border-fg 2 --no-legend "confirm, boxed and quiet"
note "confirm status $?"

printf 'one\ttwo\nthree\tfour\n' | ui table --border rounded --border-fg 6 --fullscreen --align center
note "table status $?"

printf '\n'
ui style --border double --padding "0 1" --border-fg 2 "that is all of them"
