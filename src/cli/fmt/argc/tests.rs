use super::*;

/// Aligning an aligned block is a no-op, or `--check` would never settle.
fn holds(before: &str, after: &str) {
    let once = align(before);
    assert_eq!(once, after);
    assert_eq!(align(&once), once, "aligning is not idempotent");
}

/// Every column, on the block the documentation shows.
#[test]
fn a_block_lines_up_in_columns() {
    holds(
        "# @describe Deploy a thing\n\
         # @flag -n --dry-run say what would happen\n\
         # @option -t --tries <N> how many times\n\
         # @option --verbose noisier\n\
         # @arg target! where to\n\
         # @env TOKEN! the credential\n",
        "# @describe Deploy a thing\n\
         # @flag     -n --dry-run   say what would happen\n\
         # @option   -t --tries <N> how many times\n\
         # @option      --verbose   noisier\n\
         # @arg      target!        where to\n\
         # @env      TOKEN!         the credential\n",
    );
}

/// **Padding is all it does.** The tokens and their order are what argc reads, and they come out
/// in the order they went in, however wide the gaps were.
#[test]
fn the_words_are_never_moved_only_the_gaps() {
    let before = "# @option    -t     --tries    <N>      how   many times\n";
    let after = align(before);
    assert_eq!(
        after.split_whitespace().collect::<Vec<_>>(),
        before.split_whitespace().collect::<Vec<_>>()
    );
}

/// A name is not a flag: it sits where a short flag sits, and a long one does not widen to meet it.
#[test]
fn a_long_name_does_not_push_the_flags_out() {
    holds(
        "# @flag -n --dry-run say\n\
         # @arg an-extremely-long-target-name where\n",
        "# @flag -n --dry-run                  say\n\
         # @arg  an-extremely-long-target-name where\n",
    );
}

/// argc reads a tag only at column zero, so an indented one is a comment and stays one.
#[test]
fn an_indented_tag_is_a_comment() {
    holds(
        "    # @option -t --tries <N> x\n",
        "    # @option -t --tries <N> x\n",
    );
}

/// `@describe` and `@cmd` continue onto the plain comment lines under them — that text belongs to
/// the tag above it, so a plain comment ends the block rather than joining it.
#[test]
fn a_plain_comment_ends_a_block() {
    holds(
        "# @describe Deploy\n\
         # more about deploying\n\
         # @flag -n --dry-run say\n",
        "# @describe Deploy\n\
         # more about deploying\n\
         # @flag -n --dry-run say\n",
    );
}

/// Two blocks with code between them are two blocks, and neither one's widths reach the other.
#[test]
fn blocks_are_measured_one_at_a_time() {
    holds(
        "# @cmd one\n\
         # @option --a x\n\
         one() { :; }\n\
         # @cmd two\n\
         # @option --an-extremely-long-flag y\n",
        "# @cmd    one\n\
         # @option --a x\n\
         one() { :; }\n\
         # @cmd    two\n\
         # @option --an-extremely-long-flag y\n",
    );
}

/// The `#` run is argc's too, and is kept as written.
#[test]
fn a_doubled_hash_is_still_a_tag() {
    holds("## @flag -n --dry-run say\n", "## @flag -n --dry-run say\n");
}

/// **An unknown tag is laid out as text.** Its fields have names nobody here knows, and guessing at
/// them would move words about inside a line whose meaning is somebody else's.
#[test]
fn a_tag_this_does_not_know_keeps_its_words_where_they_are() {
    holds(
        "# @describe Deploy\n\
         # @something-new a b c\n",
        "# @describe      Deploy\n\
         # @something-new a b c\n",
    );
}

/// Nothing to say is nothing to pad, or every one of these would end in trailing whitespace.
#[test]
fn a_declaration_with_no_description_has_no_trailing_space() {
    holds(
        "# @flag -q\n\
         # @option -t --tries <N>\n\
         # @describe\n",
        "# @flag     -q\n\
         # @option   -t --tries <N>\n\
         # @describe\n",
    );
}

#[test]
fn a_script_with_no_declarations_is_returned_unchanged() {
    let script = "#!/bin/sh\n# an ordinary comment\necho hi\n";
    assert_eq!(align(script), script);
    assert_eq!(align(""), "");
    // A file that did not end in a newline does not gain one here.
    assert_eq!(align("echo hi"), "echo hi");
}

#[test]
fn a_bare_at_sign_is_not_a_tag() {
    assert!(tagged("# @ nothing").is_none());
    assert!(tagged("# not a tag").is_none());
    assert!(tagged("echo @option").is_none());
    assert!(tagged("# @option -t").is_some());
}

/// A short flag is one dash and a name — anything else is a long spelling or a description.
#[test]
fn a_short_flag_is_one_dash() {
    assert!(is_short("-n"));
    assert!(is_short("+x"));
    assert!(!is_short("--dry-run"));
    assert!(!is_short("-"));
    assert!(!is_short("say"));
}
