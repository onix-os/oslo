# Styling a list: where the filter sits, what is beside it, and what colour the rows take.
#
#   oslo examples/ui_look.sh
#
# `examples/ui_chrome.sh` shows what wraps *around* a widget — the border, the legend, the screen.
# This is the other half: the rows themselves.
#
#   --look P                  a whole look under one name: plain, history, menu
#   --filter-at top|bottom    which end the query sits at
#   --reverse                 the list grows towards the filter, best match nearest
#   --slot-left / --slot-right   text beside the query: {n} {total} {index} {query} {marked}
#   --surface C               the colour under the filter row, edge to edge
#   --surface-rows N          1 is a line, 3 is a panel you type into
#   --stripe C                a quiet ruler on every other row
#   --sel-fg / --sel-bg       the row the cursor is on
#   --row-fg / --row-bg       the rest of them
#   --hit-fg / --hit-bg       the characters the query matched
#   --accent C                the marker and the prompt
#   --marker S / --prompt S / --placeholder S
#   --list-width content|full whether a row's colour stops at its text or reaches the edge
#   --list-gap N / --list-pad N
#
# The same are Lua table fields — `look`, `filter_at`, `reverse`, `slot_left`, `slot_right`,
# `surface`, `stripe`, `sel_fg`, `list_width` — on every `oslo.ui` list widget. One parser builds
# the same `Look` for both.
#
# The point of the last step is that the history browser is not a different program: it is this
# list with the filter at the bottom, the rows growing up towards it, a tinted surface under the
# query and a stripe on every other row. All of those are options, so anything can have them.

set -u

step=0
say() {
    step=$((step + 1))
    printf '\n'
    ui style --border rounded --padding "0 1" --border-fg 5 "$step/9  $1"
}
note() { ui log --level info "$1"; }
pause() {
    printf '\n'
    ui confirm --yes next --no stop "carry on?" || {
        note "stopped at step $step"
        exit 0
    }
}

# Fake history, so the last steps have something with the right shape in them.
fake_history() {
    printf '%s\n' \
        "git status" \
        "cargo build --release" \
        "cargo test -- --nocapture" \
        "make verify" \
        "nix develop --command oslo" \
        "rg --hidden --glob '!.git' TODO" \
        "docker compose up -d postgres" \
        "ssh deploy@build-01" \
        "systemctl --user restart oslo-agent" \
        "git rebase -i origin/main"
}

# --------------------------------------------------------------------- default

say "the default: filter on top, no colour, rows as wide as their text"
fake_history | ui filter --header "pick a command"
note "status $?"
pause

# ------------------------------------------------------------------ the filter

say "--filter-at bottom --reverse: the query where the cursor already is"
# The list grows *upward*, so the best match sits against the bar rather than at the far end of
# the block from it. This is the one change that turns a menu into a finder.
fake_history | ui filter --filter-at bottom --reverse --height 6
note "status $?"
pause

say "--slot-right: what the list knows about itself"
# `{n}` matched, `{total}` in the list, `{index}` where the cursor is. Without slots every widget
# that wanted a counter had to grow its own flag.
fake_history | ui filter --filter-at bottom --reverse --height 6 \
    --slot-right " {index} of {n}/{total} " --prompt "search ❯ "
note "status $?"
pause

say "--surface --surface-rows 3: somewhere to type rather than a line"
# Three rows of one colour: a blank row, the query, a blank row. The blank rows are the surface,
# not spacing around it — that is what makes it read as a panel.
fake_history | ui filter --filter-at bottom --reverse --height 6 \
    --surface 236 --surface-rows 3 --list-width full --slot-right " {n}/{total} "
note "status $?"
pause

# ------------------------------------------------------------------- the rows

say "--stripe: a quiet ruler down a long list"
# On full-width rows only. A stripe that stopped at the last letter would be a highlighted word.
fake_history | ui filter --list-width full --stripe 235 --height 8
note "status $?"
pause

say "--sel-fg --sel-bg --hit-bg: the row you are on, and the letters that put it there"
# Type something — only the characters that matched are marked. A fuzzy hit is otherwise a
# mystery: five rows come back and nothing says which letters chose them.
fake_history | ui filter --list-width full --height 8 \
    --sel-fg 0 --sel-bg 4 --hit-fg 0 --hit-bg 3 --accent 4 --marker "▸ "
note "status $?"
pause

# ----------------------------------------------------------------- the presets

say "--look menu: rows on a surface, no stripes"
ui choose --look menu --header "one block" alpha beta gamma delta
note "status $?"
pause

say "--look history: the whole combination under one name"
# Not sugar. These are the settings that have to agree with each other: a bottom filter without
# `--reverse` puts the best match furthest from the cursor, and a stripe without full-width rows
# paints a coloured word. The preset is the working combination.
fake_history | ui filter --look history --height 8
note "status $?"
pause

# ------------------------------------------------------------- and all of it

say "the history browser, rebuilt out of options"
# A preset is a starting point, so this is that look plus a border, the whole screen, and a
# counter — which is the part worth seeing. Nothing here is special-cased for history; the same
# flags work on `ui choose`, `ui file` and `ui table`.
fake_history | ui filter --look history --fullscreen --align-y bottom \
    --border rounded --border-fg 4 --border-fit full \
    --slot-left "history @ " --slot-right " {n}/{total} " \
    --placeholder "type to search" --height 10
note "status $?"

printf '\n'
ui style --border double --padding "0 1" --border-fg 2 "same widget, all of it optional"
