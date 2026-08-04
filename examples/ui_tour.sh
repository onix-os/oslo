# A tour of all thirteen `ui` widgets, in the shapes a real script would use them.
#
#   oslo examples/ui_tour.sh
#
# Each step is deliberately not the one-liner from the help text: options are combined, answers
# feed later steps, and the awkward cases are here on purpose — a list longer than the window, a
# row with a value in its third column, a filter over piped input, a command that fails.
#
# Esc cancels any widget. Every step reports the exit status, because that status *is* the
# interface: 0 answered, 1 cancelled, 2 nobody to ask.

set -u

step=0
say() {
    step=$((step + 1))
    printf '\n'
    ui style --border rounded --padding "0 1" --border-fg 5 "$step/13  $1"
}
result() {
    if [ "$2" -eq 0 ]; then
        ui log --level info "$1" --field answer="$3"
    else
        ui log --level warn "$1 — nothing chosen" --field status="$2"
    fi
}
pause() {
    printf '\n'
    ui confirm --yes "next" --no "stop here" --default "carry on?" || {
        ui log --level info "stopped at step $step"
        exit 0
    }
}

ui style --border double --padding "1 3" --bold --fg 5 "oslo ui — a tour of all thirteen"

# ---------------------------------------------------------------- 1. input
say "input — a value, with a placeholder and a starting value"
project=$(ui input --prompt "project: " --placeholder "what are you shipping" --value "oslo")
result "input" "$?" "$project"
pause

# ---------------------------------------------------------------- 2. input --password
say "input --password — the same widget, masked"
secret=$(ui input --prompt "token: " --password --placeholder "nothing is echoed")
result "password" "$?" "${secret:+(${#secret} characters)}"
pause

# ---------------------------------------------------------------- 3. write
say "write — several lines. Enter makes a new line, Ctrl-D submits"
notes=$(ui write --header "release notes for ${project:-oslo}" \
                 --placeholder "one line per change, then Ctrl-D")
status=$?
result "write" "$status" "$(printf '%s' "${notes:-}" | wc -l | tr -d ' ') line(s)"
pause

# ---------------------------------------------------------------- 4. choose
say "choose — one of several, with a header"
kind=$(ui choose --header "what kind of release is this?" \
                 patch minor major "not a release")
result "choose" "$?" "$kind"
pause

# ---------------------------------------------------------------- 5. choose --multi
say "choose --multi — space checks, enter confirms. The caret and the box are separate columns"
targets=$(ui choose --multi --header "which targets?" \
                    x86_64-linux aarch64-linux x86_64-darwin aarch64-darwin riscv64-linux)
status=$?
result "choose --multi" "$status" "$(printf '%s' "${targets:-}" | tr '\n' ' ')"
pause

# ---------------------------------------------------------------- 6. filter
say "filter — items on stdin, keys from the terminal. Type to narrow"
# More items than fit, so the window has to scroll.
verb=$(seq 1 40 | while read -r n; do printf 'commit-%03d fix a thing that was broken\n' "$n"; done \
       | ui filter --header "pick a commit" --height 8)
result "filter" "$?" "${verb%% *}"
pause

# ---------------------------------------------------------------- 7. table
say "table — columns, and the search matches ANY of them (try 'admin' or '25')"
people=$(printf 'name,age,role,team\nalice,34,admin,platform\nbob,25,user,web\ncarol,41,admin,data\ndan,29,user,platform\nerin,38,owner,data\n' \
         | ui table --header-row --height 6)
result "table" "$?" "$people"
pause

# ---------------------------------------------------------------- 8. file
say "file — Right goes in, Left goes out, Enter picks. Dotfiles included"
picked=$(ui file --all --height 10 .)
result "file" "$?" "$picked"
pause

# ---------------------------------------------------------------- 9. confirm
say "confirm — the answer is the exit status, so it composes with && and ||"
if ui confirm --yes "do it" --no "leave it" "apply ${kind:-nothing} to ${project:-oslo}?"; then
    ui log --level info "confirmed" --field kind="${kind:-none}"
else
    ui log --level warn "declined"
fi
pause

# ---------------------------------------------------------------- 10. spin
say "spin — the command's status passes through, and its stdout is still yours"
counted=$(ui spin --title "counting the corpus" -- sh -c 'sleep 1; ls tests/corpus 2>/dev/null | wc -l')
ui log --level info "spin finished" --field status=$? --field scripts="${counted:-0}"
printf '\n'
ui spin --title "this one fails" -- sh -c 'sleep 1; exit 3'
ui log --level error "and the failure comes through" --field status=$?
pause

# ---------------------------------------------------------------- 11. format
say "format — markdown, and templates"
ui format "# ${project:-oslo}

A **${kind:-patch}** release, with:

- fenced code left alone:
\`\`\`sh
echo '**not bold**'
\`\`\`
- \`inline code\` and *emphasis*
- a [link](https://github.com/bresilla/rush)

> and a quote, to finish
"
printf '\n'
ui format --type template \
          --field project="${project:-oslo}" \
          --field kind="${kind:-patch}" \
          "template: {{project}} {{kind}} — {{unset_on_purpose}} is left alone"
pause

# ---------------------------------------------------------------- 12. join
say "join — two boxes side by side, aligned. Colour does not break the alignment"
left=$(ui style --border rounded --padding "1 2" --fg 4 "chosen
────────
${kind:-patch}")
right=$(ui style --border rounded --padding "1 2" --fg 2 "targets
────────
$(printf '%s' "${targets:-none}" | head -3)")
ui join --align middle "$left" "  " "$right"
printf '\n'
ui join --vertical --align center "$(ui style --bold 'stacked')" "and centred"
pause

# ---------------------------------------------------------------- 13. pager
say "pager — full screen, then the screen comes back exactly as it was. q quits"
{
    printf '%s\n' "tour of ${project:-oslo}"
    printf '%s\n' "================================"
    printf '\n'
    printf 'a long line that is wider than most terminals and therefore exercises the wrapping code path rather than the truncating one, which is the whole reason it is this long\n'
    printf '\n'
    seq 1 200 | while read -r n; do printf '%4d  line %d of the paged document\n' "$n" "$n"; done
} | ui pager --title "everything above, paged" --wrap
ui log --level info "pager closed" --field status=$?

# ---------------------------------------------------------------- done
printf '\n'
ui style --border double --padding "1 3" --fg 2 --bold "that was all thirteen"
ui log --level info "tour complete" \
       --field project="${project:-?}" \
       --field kind="${kind:-?}" \
       --field targets="$(printf '%s' "${targets:-none}" | tr '\n' ',')"
