use super::substitute;
use std::collections::HashMap;

/// Substitute with a fixed alias table, as if the environment already held it.
fn with(aliases: &[(&str, &str)], source: &str) -> String {
    let table: HashMap<String, String> = aliases
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
    substitute(source, &|name| table.get(name).cloned())
}

/// Nothing to substitute must come back byte-for-byte, whatever the text contains.
fn unchanged(source: &str) {
    assert_eq!(
        with(&[], source),
        source,
        "text was rewritten with no aliases"
    );
}

#[test]
fn a_command_word_is_replaced() {
    assert_eq!(with(&[("ll", "ls -la")], "ll"), "ls -la");
    assert_eq!(with(&[("ll", "ls -la")], "ll /tmp"), "ls -la /tmp");
}

/// The defect this module exists for: an alias body is source text and may open a construct the
/// word it replaced could not.
#[test]
fn an_alias_may_contribute_syntax() {
    let out = with(
        &[("forever", "while :; do")],
        "forever\n  echo hi\n  break\ndone\n",
    );
    assert_eq!(out, "while :; do\n  echo hi\n  break\ndone\n");
}

/// Only in command position: an alias name used as an argument is an argument.
#[test]
fn an_operand_is_not_substituted() {
    assert_eq!(with(&[("ll", "ls -la")], "echo ll"), "echo ll");
    assert_eq!(with(&[("ll", "ls -la")], "grep ll file"), "grep ll file");
    // ...but after a separator a command begins again.
    assert_eq!(with(&[("ll", "ls -la")], "echo x; ll"), "echo x; ls -la");
    assert_eq!(
        with(&[("ll", "ls -la")], "echo x && ll"),
        "echo x && ls -la"
    );
    assert_eq!(with(&[("ll", "ls -la")], "echo x | ll"), "echo x | ls -la");
}

/// A command prefix does not consume the command position.
#[test]
fn an_assignment_prefix_keeps_the_command_position() {
    assert_eq!(with(&[("ll", "ls -la")], "LC_ALL=C ll"), "LC_ALL=C ls -la");
}

#[test]
fn reserved_words_open_a_command_position() {
    assert_eq!(
        with(&[("ll", "ls -la")], "if ll; then ll; fi"),
        "if ls -la; then ls -la; fi"
    );
    assert_eq!(
        with(&[("ll", "ls -la")], "while ll; do ll; done"),
        "while ls -la; do ls -la; done"
    );
}

/// A chain resolves, and a self-reference stops. `alias ls='ls -F'` is the reason the second half
/// matters: without the active-name set it would not terminate.
#[test]
fn chains_resolve_and_self_reference_terminates() {
    assert_eq!(
        with(&[("ll", "ls -la"), ("ls", "echo LS")], "ll"),
        "echo LS -la"
    );
    assert_eq!(with(&[("ls", "ls -F")], "ls"), "ls -F");
    assert_eq!(with(&[("a", "b"), ("b", "a")], "a"), "a");
}

/// An alias body ending in a blank makes the *next* word a candidate too.
#[test]
fn a_trailing_blank_extends_substitution_to_the_next_word() {
    // Blanks are not preserved exactly — the body brings its own — so compare on words.
    let out = with(&[("sudo", "sudo "), ("ll", "ls -la")], "sudo ll");
    assert_eq!(
        out.split_whitespace().collect::<Vec<_>>(),
        ["sudo", "ls", "-la"]
    );
    // Without the blank it stops at one word.
    assert_eq!(
        with(&[("sudo", "sudo"), ("ll", "ls -la")], "sudo ll"),
        "sudo ll"
    );
}

/// A script may define an alias and use it further down, which is what bash allows by parsing one
/// command at a time — and the line it was defined on is *not* one of them.
#[test]
fn definitions_in_the_text_take_effect_on_the_next_line() {
    assert_eq!(
        with(&[], "alias ll='ls -la'\nll\n"),
        "alias ll='ls -la'\nls -la\n"
    );
    // Same line: bash reports `x: command not found`, so neither shell substitutes here.
    assert_eq!(with(&[], "alias x='echo hi'; x"), "alias x='echo hi'; x");
    assert_eq!(
        with(&[], "alias a='echo A'\nalias b='a'\nb\n"),
        "alias a='echo A'\nalias b='a'\necho A\n"
    );
}

#[test]
fn quoted_and_commented_text_is_left_alone() {
    unchanged("echo 'll'\n");
    assert_eq!(with(&[("ll", "ls")], "echo 'll'"), "echo 'll'");
    assert_eq!(with(&[("ll", "ls")], "echo \"ll\""), "echo \"ll\"");
    assert_eq!(with(&[("ll", "ls")], "# ll is nice"), "# ll is nice");
    assert_eq!(with(&[("ll", "ls")], "echo x # ll"), "echo x # ll");
    // A quoted name is not a name.
    assert_eq!(with(&[("ll", "ls")], "'ll'"), "'ll'");
    assert_eq!(with(&[("ll", "ls")], "\\ll"), "\\ll");
}

/// A here-document body is data. `config.guess` writes a C program through one.
#[test]
fn heredoc_bodies_are_data() {
    let source = "cat <<EOF\nll\nforever\nEOF\nll\n";
    assert_eq!(
        with(&[("ll", "ls -la"), ("forever", "while :; do")], source),
        "cat <<EOF\nll\nforever\nEOF\nls -la\n"
    );
    // The tab-stripping spelling too.
    let source = "cat <<-END\n\tll\n\tEND\nll\n";
    assert_eq!(
        with(&[("ll", "ls")], source),
        "cat <<-END\n\tll\n\tEND\nls\n"
    );
}

/// A function's name is not a command being run, so defining `ll()` must not rewrite it into
/// `ls -la()` — which does not parse.
#[test]
fn a_function_definition_is_not_substituted() {
    assert_eq!(with(&[("ll", "ls -la")], "ll() { :; }"), "ll() { :; }");
    assert_eq!(with(&[("ll", "ls -la")], "ll () { :; }"), "ll () { :; }");
    assert_eq!(
        with(&[("ll", "ls -la")], "ll (  ) { :; }"),
        "ll (  ) { :; }"
    );
}

/// A word followed by a *non-empty* subshell is a command with a subshell after it, not a function
/// definition. modernish's `alias not='! '` is used as `not (readonly foo; …)`, and reading that
/// as a definition of a function called `not` left the text a syntax error — which is exactly what
/// it is until the alias expands.
#[test]
fn a_word_before_a_subshell_is_still_substituted() {
    // Compared on words: the body brings its own trailing blank, so the spacing is not preserved
    // exactly. What matters is that `not` became `!` and the subshell survived.
    let a = &[("not", "! "), ("ll", "ls -la")];
    let words = |s: String| s.split_whitespace().map(str::to_string).collect::<Vec<_>>();
    assert_eq!(words(with(a, "not (exit 0)")), ["!", "(exit", "0)"]);
    assert_eq!(
        words(with(a, "true && not (exit 0)")),
        ["true", "&&", "!", "(exit", "0)"]
    );
    assert_eq!(words(with(a, "not ( : )")), ["!", "(", ":", ")"]);
}

/// A redirection operand is a filename.
#[test]
fn a_redirection_target_is_not_a_command() {
    assert_eq!(with(&[("ll", "ls")], "echo x > ll"), "echo x > ll");
    assert_eq!(with(&[("ll", "ls")], "cat < ll"), "cat < ll");
}

/// Text with no aliases in play must survive exactly, including the awkward corners.
#[test]
fn text_without_aliases_is_untouched() {
    unchanged("echo 'it'\\''s'\n");
    unchanged("x=$(cat <<'EOF'\nbody\nEOF\n)\n");
    unchanged("case $x in\n  a) echo a ;;\n  *) echo b ;;\nesac\n");
    unchanged("printf '%s\\n' \"a b\" # comment with 'quote\n");
    unchanged("");
    unchanged("\n\n");
    unchanged("echo no-trailing-newline");
}

/// `case` patterns are words, not commands: substituting there would rewrite the thing being
/// matched against.
#[test]
fn case_patterns_are_not_commands() {
    let source = "case $x in\n  ll) echo hit ;;\nesac\n";
    // The pattern keeps its text; only the body is a command position.
    let out = with(&[("ll", "ls -la"), ("echo", "printf")], source);
    assert!(out.contains("ll)"), "the pattern was rewritten: {out}");
}

/// The scanner must not loop or panic on input designed to confuse it.
#[test]
fn hostile_input_terminates() {
    let deep: String = (0..40)
        .map(|i| format!("alias a{i}='a{}'\n", i + 1))
        .collect();
    let source = format!("{deep}a0\n");
    let out = substitute(&source, &|_| None);
    assert!(out.contains("a0") || out.contains("a39"));

    unchanged("echo \"unterminated\n");
    unchanged("echo 'unterminated\n");
    unchanged("cat <<EOF\nnever closed\n");
}

/// `$(( … ))` is arithmetic and `${ … }` is a parameter expansion: neither holds commands, and
/// substituting in them corrupts the expression. `alias n=…` over `$(( n + 1 ))` produced
/// `$(( echo BAD + 1 ))`, which is a fatal expansion error rather than a wrong answer.
#[test]
fn expansions_that_are_not_command_text_are_left_alone() {
    let a = &[("n", "echo BAD"), ("x", "echo BAD")];
    assert_eq!(with(a, "echo $(( n + 1 ))"), "echo $(( n + 1 ))");
    assert_eq!(with(a, "echo $((n+1))"), "echo $((n+1))");
    assert_eq!(with(a, "echo $(( (n) * 2 ))"), "echo $(( (n) * 2 ))");
    assert_eq!(with(a, "echo ${x}"), "echo ${x}");
    assert_eq!(with(a, "echo ${x:-n}"), "echo ${x:-n}");
    assert_eq!(with(a, "echo ${x#*/}"), "echo ${x#*/}");
    // An arithmetic *command* is the same story.
    assert_eq!(with(a, "(( n = 1 ))"), "(( n = 1 ))");
}

/// A command substitution *is* shell text, but this pass must not rewrite it: its body is kept as
/// source and parsed — through this same pass — when the substitution runs. Substituting here as
/// well applied every alias twice, which turned modernish's `alias let='let --'` into
/// `let -- -- "…"` and killed every arithmetic test it makes.
#[test]
fn command_substitutions_are_left_for_their_own_parse() {
    let a = &[("ll", "ls -la")];
    assert_eq!(with(a, "echo $(ll)"), "echo $(ll)");
    assert_eq!(with(a, "echo $( ll )"), "echo $( ll )");
    assert_eq!(with(a, "echo `ll`"), "echo `ll`");
    assert_eq!(with(a, "x=$(ll); ll"), "x=$(ll); ls -la");
    // Nesting and quoting inside must not confuse the copy.
    assert_eq!(
        with(a, "echo $(echo $(ll) \")\")"),
        "echo $(echo $(ll) \")\")"
    );
}

/// An unterminated expansion is a syntax error for the parser to report; this pass must neither
/// hang on it nor rewrite past it.
#[test]
fn unterminated_expansions_terminate() {
    unchanged("echo $(( 1 + 1\n");
    unchanged("echo ${x\n");
    unchanged("echo $(\n");
}

/// A `for`/`select` word list holds words, not commands — but it ends at the `;` or newline
/// before `do`, and what follows *is* a command position. modernish writes its loops as
/// `LOOP for i in 1 to 10; DO … DONE`, so a list that stayed open swallowed the `DO` and then
/// every alias in the rest of the file.
#[test]
fn a_for_word_list_ends_at_the_separator() {
    let a = &[("ll", "ls -la"), ("DO", "do"), ("DONE", "done")];
    // The list itself is words.
    assert_eq!(
        with(a, "for x in ll; do :; done"),
        "for x in ll; do :; done"
    );
    // What follows the separator is not.
    assert_eq!(
        with(a, "for x in 1 2; DO ll; DONE"),
        "for x in 1 2; do ls -la; done"
    );
    assert_eq!(
        with(a, "for x in 1 2\nDO\n  ll\nDONE"),
        "for x in 1 2\ndo\n  ls -la\ndone"
    );
    // A `case` pattern list, by contrast, survives a newline.
    assert_eq!(
        with(a, "case $x in\n  ll) :; ;;\nesac"),
        "case $x in\n  ll) :; ;;\nesac"
    );
}

/// A trailing backslash inside an expansion used to push the copy index one past the end of the
/// text, and slicing with it **panicked the shell**. modernish's signal module contains one.
#[test]
fn a_trailing_backslash_in_an_expansion_does_not_panic() {
    unchanged("echo $(cmd \\");
    unchanged("echo ${x\\");
    unchanged("echo $(( 1 \\");
    unchanged("echo $(\"a\\");
    unchanged("x=$(a|b\\");
    // The real shape it came from: a `\\|` inside a command substitution near the end of input.
    unchanged("c=${c:-x}${n}\\|${s}$(f \\");
}

/// A command substitution written across lines is still one construct. The copy used to stop at
/// the end of the line, so the rest of its body was scanned as ordinary text — and since that body
/// is parsed again when the substitution runs, every alias in it was substituted twice. modernish
/// builds its signal table inside exactly such a substitution, and `alias let='let --'` became
/// `let -- --`.
#[test]
fn a_multiline_command_substitution_is_copied_whole() {
    let a = &[("let", "let --"), ("ll", "ls -la")];
    let source = "o=$(\n\ti=0\n\twhile let \"(i+=1)<4\"; do\n\t\tll\n\tdone\n)\nll\n";
    assert_eq!(
        with(a, source),
        "o=$(\n\ti=0\n\twhile let \"(i+=1)<4\"; do\n\t\tll\n\tdone\n)\nls -la\n"
    );
    // The same for `${ … }` and for an arithmetic expansion.
    assert_eq!(
        with(a, "x=${v:-\n  ll\n}\nll\n"),
        "x=${v:-\n  ll\n}\nls -la\n"
    );
    assert_eq!(
        with(a, "x=$((\n 1 +\n 1 ))\nll\n"),
        "x=$((\n 1 +\n 1 ))\nls -la\n"
    );
}

/// A comment inside a `$( … )` being copied through is not shell. A lone apostrophe in one opened
/// a quote that swallowed the rest of the construct, so the `)` closing it was never seen and
/// every alias after it went unsubstituted. modernish's `builtin.t` carries exactly that comment.
#[test]
fn a_comment_inside_a_copied_construct_is_not_shell() {
    let a = &[("ll", "ls -la")];
    let source = "v=$(\n\t: # many shells don't check here\n\techo hi\n)\nll\n";
    assert_eq!(
        with(a, source),
        "v=$(\n\t: # many shells don't check here\n\techo hi\n)\nls -la\n"
    );
    // An unbalanced paren or double quote in a comment is just as harmless.
    let source = "v=$(\n\t: # a stray ( and a \"\n\techo hi\n)\nll\n";
    assert_eq!(
        with(a, source),
        "v=$(\n\t: # a stray ( and a \"\n\techo hi\n)\nls -la\n"
    );
    // A `#` that is not at a word boundary is not a comment, and `${v#pat}` must survive.
    assert_eq!(with(a, "v=$(echo a#b)\nll\n"), "v=$(echo a#b)\nls -la\n");
    assert_eq!(
        with(a, "v=$(echo ${x#y})\nll\n"),
        "v=$(echo ${x#y})\nls -la\n"
    );
    // A quoted `#` is data, not a comment.
    assert_eq!(with(a, "v=$(echo '#')\nll\n"), "v=$(echo '#')\nls -la\n");
}
