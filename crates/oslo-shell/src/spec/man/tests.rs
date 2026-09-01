use super::*;

/// A page, as `man -P cat` renders one: headings in column zero, everything else indented.
const LS: &str = "\
LS(1)                            User Commands                           LS(1)

NAME
       ls - list directory contents

DESCRIPTION
       List information about the FILEs.

OPTIONS
       -a, --all
              do not ignore entries starting with .  There is more here.

       -w, --width=COLS
              set output width to COLS

       -o FILE
              like -l, but do not list group information

       --color[=WHEN]
              colorize the output

       -1     list one file per line

       --help Output a usage message and exit.

       -b, --backup
              check device numbers when creating incre-
              mental archives (de- fault)

SEE ALSO
       --not-an-option
";

fn ls() -> CommandSpec {
    from_page("ls", LS).expect("the sample page yields flags")
}

fn flag<'a>(spec: &'a CommandSpec, name: &str) -> &'a OptionSpec {
    spec.options
        .iter()
        .find(|option| option.names.iter().any(|had| had == name))
        .unwrap_or_else(|| panic!("no {name} in {:?}", spec.options))
}

/// Bold is `X\bX` and italic is `_\bX`. Left in, every flag is spelled `--aallll`.
#[test]
fn the_overstrike_typography_comes_back_off() {
    assert_eq!(plain("-\u{8}--\u{8}-a\u{8}al\u{8}ll\u{8}l"), "--all");
    assert_eq!(plain("_\u{8}F_\u{8}I_\u{8}L_\u{8}E"), "FILE");
    assert_eq!(plain("plain text"), "plain text");
}

/// Every spelling of one flag, and only the flags.
#[test]
fn a_page_gives_up_its_flags() {
    let spec = ls();
    assert_eq!(spec.name, "ls");
    assert_eq!(flag(&spec, "-a").names, ["-a", "--all"]);
    assert_eq!(flag(&spec, "--width").names, ["-w", "--width"]);
    assert_eq!(flag(&spec, "-1").names, ["-1"]);
}

/// The line under `SEE ALSO` looks exactly like a flag and is not one — the section heading is the
/// only thing that says so.
#[test]
fn only_the_options_sections_are_read() {
    let spec = ls();
    assert!(
        !spec
            .options
            .iter()
            .any(|option| option.names.iter().any(|name| name == "--not-an-option")),
        "a flag was taken from a section that is not about options"
    );
}

/// Whether the next word belongs to the flag decides how the whole line is parsed after it.
#[test]
fn a_flag_that_takes_a_value_says_so() {
    let spec = ls();
    assert_eq!(flag(&spec, "--width").takes, Arg::Required);
    assert_eq!(flag(&spec, "-o").takes, Arg::Required);
    assert_eq!(flag(&spec, "--color").takes, Arg::Required);
    // A switch, and the word after it is the command's own argument.
    assert_eq!(flag(&spec, "-a").takes, Arg::None);
    assert_eq!(flag(&spec, "-1").takes, Arg::None);
}

/// The prose under the flag when the line carried none, the prose on the line when it did.
#[test]
fn a_description_is_one_sentence_from_wherever_it_was() {
    let spec = ls();
    assert_eq!(
        flag(&spec, "-a").description,
        "do not ignore entries starting with ."
    );
    assert_eq!(flag(&spec, "-1").description, "list one file per line");
    assert_eq!(
        flag(&spec, "--width").description,
        "set output width to COLS"
    );
}

/// **One space is a description too.** `grep` writes `--help Output a usage message and exit.`,
/// and a rule that only knew about the wide gap read the whole sentence as signature.
#[test]
fn a_description_one_space_from_its_flag_is_still_a_description() {
    assert_eq!(
        flag(&ls(), "--help").description,
        "Output a usage message and exit"
    );
}

/// A word `man` broke across the margin is put back together — but `--` at a line end is not a
/// broken word, and welding it to the next one would invent a flag.
#[test]
fn a_word_split_at_the_margin_is_rejoined() {
    assert_eq!(
        flag(&ls(), "-b").description,
        "check device numbers when creating incremental archives (de- fault)"
    );
    assert_eq!(
        prose_under(&["  -x", "      print -- ", "      between them"], 0),
        "print -- between them"
    );
}

/// **The failure mode is no completion, not a wrong one.** Prose that happens to begin with a dash
/// must not become a flag, and a page that yields one lonely flag yielded it by accident.
#[test]
fn a_page_with_nothing_in_it_offers_nothing() {
    assert!(from_page("x", "NAME\n       x - a thing\n").is_none());
    assert!(from_page("x", "OPTIONS\n       nothing here starts with a dash\n").is_none());
    assert!(from_page("x", "OPTIONS\n       -a     the only one\n").is_none());
    assert!(from_page("x", "OPTIONS\n       -- ends the options\n").is_none());
}

/// A word of English is not a placeholder, or every switch swallows the word after it.
#[test]
fn a_placeholder_is_upper_case_or_bracketed() {
    assert!(is_a_placeholder("FILE"));
    assert!(is_a_placeholder("<path>"));
    assert!(is_a_placeholder("NUM"));
    assert!(!is_a_placeholder("do"));
    assert!(!is_a_placeholder("entries"));
    assert!(!is_a_placeholder(""));
    assert!(!is_a_placeholder("123"));
}

#[test]
fn a_flag_is_one_or_two_dashes_and_a_name() {
    assert!(is_a_flag("-a"));
    assert!(is_a_flag("--all"));
    assert!(is_a_flag("--dry-run"));
    assert!(!is_a_flag("-"));
    assert!(!is_a_flag("--"));
    assert!(!is_a_flag("---x"));
    assert!(!is_a_flag("-."));
    assert!(!is_a_flag("word"));
}

/// A word with a path in it is a path, and handing one to `man` would run it on whatever was half
/// typed.
#[test]
fn only_something_that_could_be_a_command_is_looked_up() {
    assert!(!is_a_command_name(""));
    assert!(!is_a_command_name("/bin/ls"));
    assert!(!is_a_command_name("./x"));
    assert!(!is_a_command_name("-a"));
    assert!(!is_a_command_name("a b"));
    assert!(is_a_command_name("ls"));
    assert!(is_a_command_name("python3.11"));
    assert!(is_a_command_name("g++"));
}

/// Against whatever `man` this machine has, which is the only way to find out that a real page
/// still looks like the sample above.
///
/// Ignored by default: a build machine may have no man pages at all, and a test that fails there
/// is a test that gets deleted.
///
/// ```sh
/// cargo test -p oslo-shell --features compgen man -- --ignored --nocapture
/// ```
#[test]
#[ignore = "needs man pages installed"]
fn a_real_page_on_this_machine() {
    for command in ["ls", "grep", "find", "tar", "curl", "git"] {
        match spec(command) {
            Some(spec) => {
                println!("\n{command}: {} flags", spec.options.len());
                for option in spec.options.iter().take(8) {
                    println!("  {:<24} {}", option.names.join(", "), option.description);
                }
            }
            None => println!("\n{command}: nothing"),
        }
    }
}

#[test]
fn a_description_is_trimmed_to_a_line() {
    assert_eq!(shorten("one. two"), "one");
    assert_eq!(shorten("  spaced  "), "spaced");
    let long = "x".repeat(200);
    assert!(shorten(&long).chars().count() <= 91);
}
