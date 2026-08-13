//! Completing a script from the comments that declare its arguments.
//!
//! ```text
//! deploy --<Tab>        →  --dry-run  --tries  --help
//! ```
//!
//! for any script on `$PATH` whose comments say so, and for any script in the macro database — with
//! no completion file to generate, install or keep in step. argc ships generated completions for
//! nine shells to reach this; oslo calls the function.
//!
//! # Two cheap questions before the expensive one
//!
//! Completion runs on a keystroke, so the order matters: a name is resolved and its file read only
//! after the *word* has been looked at, and `argc::compgen` runs only once the source has been seen
//! to contain a declaration at all. A machine with no argc-shaped scripts pays one `read` on the
//! first Tab against a given command and nothing after it.

use oslo_ui::completion::provider::{Ctx, Offer};

/// Offers for the word being completed, or nothing at all.
pub fn offers(ctx: &Ctx) -> Vec<Offer> {
    let Some(source) = source_of(&ctx.command) else {
        return Vec::new();
    };
    // **The cheapest test that a script is argc-shaped.** Every declaration is a `# @` comment, and
    // a script without one has nothing for the parser to find.
    if !source.contains("# @") {
        return Vec::new();
    }

    let mut env = crate::env::Environment::new();
    let runtime = super::Shell::new(&mut env);
    // What `compgen` matches: the command, the words already typed, and the partial word last —
    // empty when the cursor sits after a space, which is how it knows to offer everything.
    let mut words = ctx.words.clone();
    words.push(ctx.current.clone());

    let mut offers = ask(runtime, &ctx.command, &source, &words);

    // **The options, even when nothing has been typed yet.** `compgen` answers an empty word with
    // the *positional* it expects and nothing else — the row for `deploy <Tab>` was
    // `__argc_value=BUILD`, an instruction to a completion script rather than a word anybody types.
    // A person pressing Tab on a command wants to see what it takes, and what it takes includes its
    // flags. So the same question is asked again with a `-`, which is how argc is told to list them.
    if ctx.current.is_empty() {
        let mut dashed = words.clone();
        *dashed.last_mut().expect("current was pushed") = "-".to_string();
        offers.extend(ask(runtime, &ctx.command, &source, &dashed));
    }
    offers
}

/// One `compgen` call, with the rows a person could not type filtered out.
fn ask(runtime: super::Shell, command: &str, source: &str, words: &[String]) -> Vec<Offer> {
    argc::compgen(runtime, argc::Shell::Generic, command, source, words, true)
        .unwrap_or_default()
        .lines()
        .filter_map(one)
        .collect()
}

/// One line of what `compgen` printed.
///
/// **Three fields, not two**: the value, a display hint, then the description —
///
/// ```text
/// --dry-run\t/color:cyan\tsay what would happen
/// staging\t/color:default
/// ```
///
/// The hint is for a completion *script* deciding how to paint a row, which oslo's dropdown does
/// itself from its own theme. Splitting on the first tab alone put `/color:cyan` at the front of
/// every description — visible in the menu, and the sort of detail that is only ever found by
/// looking at the output rather than at the documentation.
fn one(line: &str) -> Option<Offer> {
    let mut fields = line.split('\t');
    let display = fields.next()?.trim();
    if display.is_empty() {
        return None;
    }
    // **`__argc_value=BUILD` is not a word.** It is `compgen` telling a completion *script* to
    // complete a value of that kind here — a file, a directory, whatever the declaration said.
    // oslo's dropdown already completes files itself, so the marker is dropped rather than offered
    // as something to type, which is what it was: the row for `deploy <Tab>` said `__argc_value`.
    if display.starts_with("__argc") {
        return None;
    }
    let description = fields
        .filter(|field| !field.starts_with('/'))
        .map(str::trim)
        .filter(|field| !field.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    Some(Offer {
        display: display.to_string(),
        description: (!description.is_empty()).then_some(description),
    })
}

/// The text of the script `name` runs — **resolved the way running it would resolve it**.
///
/// `$PATH` first, then the macro store, because that is the order `exec::stored` uses: a stored
/// script is found only after the `$PATH` search has failed, so nothing on disk can be shadowed.
/// Reading them the other way round is not a cosmetic difference — with a `deploy` on `$PATH` *and*
/// a `deploy` in the database, completion described one script and Tab ran the other, which is how
/// a bare `deploy <Tab>` came to offer nothing at all: the stored one had no declarations in it.
fn source_of(name: &str) -> Option<String> {
    if let Some(path) = crate::env::builtins::hash_lookup(name)
        && let Ok(source) = std::fs::read_to_string(path)
    {
        return Some(source);
    }
    use argc::Runtime;
    let mut env = crate::env::Environment::new();
    super::Shell::new(&mut env).read_to_string(name)
}

/// Whether a word could name an argc-shaped script at all.
///
/// Asked before anything is read: an offer for `git` or `ls` is never coming from here, and a Tab
/// after a command with no declarations should cost nothing.
pub fn worth_asking(command: &str) -> bool {
    !command.is_empty() && !command.contains('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_becomes_an_offer_with_its_description() {
        // The real shape, three fields: value, display hint, description.
        let offer = one("--tries\t/color:cyan\thow many times").expect("an offer");
        assert_eq!(offer.display, "--tries");
        assert_eq!(
            offer.description.as_deref(),
            Some("how many times"),
            "the colour hint is the dropdown's business, not a description"
        );

        // A value has a hint and nothing else to say.
        let value = one("staging\t/color:default").expect("an offer");
        assert_eq!(value.display, "staging");
        assert_eq!(value.description, None);

        let bare = one("--force").expect("an offer");
        assert_eq!(bare.display, "--force");
        assert_eq!(bare.description, None);

        assert!(one("").is_none(), "a blank line is not an offer");
        assert!(one("   ").is_none());
    }

    /// **`__argc_value=BUILD` is an instruction, not a word.** It was offered as a row: pressing
    /// Tab on a bare `deploy` put `__argc_value=BUILD` in the menu and no flags at all.
    #[test]
    fn an_internal_marker_is_never_offered() {
        assert!(one("__argc_value=BUILD").is_none());
        assert!(one("__argc_value\t/color:default").is_none());
        assert!(one("--dry-run\t/color:cyan\tsay what").is_some());
    }

    #[test]
    fn only_a_bare_command_word_is_worth_reading_a_file_for() {
        assert!(worth_asking("deploy"));
        assert!(!worth_asking(""));
        assert!(!worth_asking("./deploy"), "a path is not a stored macro");
    }

    /// **Every option, even before a dash is typed.** `compgen` answers an empty word with the
    /// positional it expects and nothing else, so a bare `deploy` used to offer no flags — and
    /// what somebody pressing Tab on a command wants first is to see what it takes.
    #[test]
    fn a_bare_command_offers_its_options() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("deploy");
        std::fs::write(
            &path,
            "# @flag   -n --dry-run   say what would happen\n# @arg build!\n",
        )
        .expect("write");

        let ctx = Ctx {
            command: path.to_string_lossy().into_owned(),
            words: vec![path.to_string_lossy().into_owned()],
            current: String::new(),
            arg: 1,
            cwd: dir.path().display().to_string(),
        };
        // Read through the same door the provider uses, so this exercises the empty-word branch
        // rather than the file lookup.
        let source = std::fs::read_to_string(&path).expect("read");
        let mut env = crate::env::Environment::new();
        let runtime = super::super::Shell::new(&mut env);
        let mut words = ctx.words.clone();
        words.push(String::new());
        let bare = ask(runtime, &ctx.command, &source, &words);
        assert!(
            bare.iter().all(|o| !o.display.starts_with("__argc")),
            "a marker reached the menu: {bare:?}"
        );

        *words.last_mut().expect("pushed") = "-".to_string();
        let dashed = ask(runtime, &ctx.command, &source, &words);
        let shown: Vec<&str> = dashed.iter().map(|o| o.display.as_str()).collect();
        assert!(shown.contains(&"--dry-run"), "{shown:?}");
    }

    /// The whole path, on a script that declares its arguments.
    #[test]
    fn a_declared_option_is_offered() {
        let source = "\
# @describe Deploy
# @flag   -n --dry-run    say what would happen
# @option -t --tries <N>  how many times
";
        let mut env = crate::env::Environment::new();
        let runtime = super::super::Shell::new(&mut env);
        let words = vec!["demo".to_string(), "--".to_string()];
        let text = argc::compgen(runtime, argc::Shell::Generic, "demo", source, &words, true)
            .expect("compgen");
        let offers: Vec<Offer> = text.lines().filter_map(one).collect();
        let shown: Vec<&str> = offers.iter().map(|o| o.display.as_str()).collect();
        assert!(shown.contains(&"--dry-run"), "{shown:?}");
        assert!(shown.contains(&"--tries"), "{shown:?}");
    }
}
