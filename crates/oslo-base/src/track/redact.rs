//! What may be written down, and how much of it.
//!
//! This store records the command line *and* the directory it ran in, and the directory is often
//! the more identifying half — a client's name, an employer's, an unreleased project. The
//! mechanisms that answer that are layered so that no single one has to be perfect.
//!
//! The two the user controls are elsewhere and come first: a leading space still means "do not
//! record this", and a non-interactive shell has no store at all. What is left here is the
//! structural mitigation and the filters.
//!
//! # The structural mitigation
//!
//! A secret is almost never the command *name*; it is in the arguments. So when a line trips any
//! filter, the row is still written — with `argv` reduced to [`head_of`] alone. The directory, the
//! run count, the timing and the exit status all survive; only the risky text is dropped. A
//! denylist that has to be perfect is a bad design. A denylist whose worst failure is reduced
//! resolution is a fine one, and that is what makes it acceptable that the list below is
//! incomplete — it always will be.

use regex::Regex;
use std::sync::OnceLock;

/// The longest line worth remembering.
///
/// Past this it is a paste rather than a habit: it will never be suggested, it is the shape a
/// leaked key arrives in, and storing it would put a kilobyte of one-off text in a table whose
/// whole argument is that it is bounded by distinct behaviour.
const MAX_ARGV: usize = 4096;

/// Words that wrap another command rather than being one.
///
/// Grouping everything a user does under `sudo` makes the per-tool timing table a joke.
const WRAPPERS: &[&str] = &[
    "builtin", "command", "doas", "env", "ionice", "nice", "nohup", "sudo", "time",
];

/// Wrapper options that take a separate value, so the value is not mistaken for the command.
///
/// Without this, `sudo -u root cargo build` groups under `root`. Long options are skipped whole
/// because the glued `--user=root` form is the common one.
const VALUE_FLAGS: &[&str] = &["-C", "-U", "-c", "-g", "-n", "-p", "-u"];

/// Tools whose first argument names what they are actually doing.
///
/// `cargo build` and `cargo test` are not the same activity and must not share a timing row, which
/// is the whole reason `head` exists as a column. The alternative — a list of *verbs* — was
/// rejected: it groups `ls build` as a build.
const SUBCOMMANDS: &[&str] = &[
    "apt",
    "apt-get",
    "aws",
    "brew",
    "bun",
    "bundle",
    "cargo",
    "composer",
    "conda",
    "deno",
    "dnf",
    "docker",
    "gcloud",
    "gem",
    "gh",
    "git",
    "go",
    "helm",
    "ip",
    "kubectl",
    "nix",
    "nmcli",
    "npm",
    "pacman",
    "pip",
    "pip3",
    "pnpm",
    "podman",
    "poetry",
    "port",
    "rustup",
    "systemctl",
    "terraform",
    "tmux",
    "uv",
    "yarn",
    "zfs",
    "zpool",
];

/// Commands whose arguments are, by their nature, credentials.
const SECRET_COMMANDS: &[&str] = &[
    "gpg",
    "htpasswd",
    "keyctl",
    "mysql",
    "op",
    "openssl",
    "pass",
    "psql",
    "secret-tool",
    "security",
    "ssh-add",
    "vault",
];

/// Subcommands that hand a tool a credential.
const SECRET_SUBCOMMANDS: &[(&str, &str)] = &[
    ("aws", "configure"),
    ("aws", "sso"),
    ("docker", "login"),
    ("gh", "auth"),
    ("npm", "adduser"),
    ("npm", "login"),
    ("npm", "token"),
];

/// Variable names that say what they hold.
const SECRET_NAMES: &[&str] = &["KEY", "PASS", "SECRET", "TOKEN"];

/// Option names whose value is a credential, in every spelling anyone writes them.
fn secret_option() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(
            r"(?i)^--?(password|passwd|token|secret|api[-_]?key|auth|bearer|credential)([=:].*)?$",
        )
        .expect("a literal pattern")
    })
}

/// The interesting part of a command line: what you ran, not how you launched it.
///
/// Leading `VAR=value` assignments and wrapper words come off, and a tool whose first argument is a
/// subcommand keeps two words. `sudo cargo build --release` is `cargo build`.
///
/// Only the first physical line is read: a command continued over several lines is one entry with
/// newlines in it, and its head is on the first of them.
pub fn head_of(line: &str) -> String {
    let words: Vec<&str> = first_command(line.lines().next().unwrap_or_default())
        .split_whitespace()
        .collect();

    let mut at = 0;
    while at < words.len() {
        if is_assignment(words[at]) {
            at += 1;
            continue;
        }
        if WRAPPERS.contains(&words[at]) {
            match after_wrapper(&words, at + 1) {
                // A wrapper with nothing after it is the command. `time` on its own is `time`.
                Some(next) => at = next,
                None => break,
            }
            continue;
        }
        break;
    }

    let Some(&command) = words.get(at) else {
        return String::new();
    };
    match words.get(at + 1) {
        Some(&second) if SUBCOMMANDS.contains(&command) && is_subcommand(second) => {
            format!("{command} {second}")
        }
        _ => command.to_string(),
    }
}

/// The characters that end one command and begin another, or send its output somewhere.
const SEPARATORS: [char; 8] = [';', '&', '|', '(', ')', '<', '>', '`'];

/// As much of a line as belongs to the command it starts with.
///
/// Splitting on whitespace alone leaves a separator glued to the word beside it, and the head is
/// then a string no other line will ever produce: `cd; pwd` gave a head of `cd;`, filed apart from
/// every other `cd` the user has ever run. Since the whole reason `head` is a column is that "what
/// does this tool cost me" should be one grouped read, a punctuation mark that splits a tool into
/// two groups defeats it.
///
/// Leading separators come off first — `(cd build && make)` opens with one, and cutting at it would
/// leave nothing and drop the row entirely rather than merely mis-file it.
///
/// Quoting is not tracked, deliberately. Cutting `git commit -m "wip; fix"` at the quoted `;` shortens
/// a line whose first two words this has already read, and those are all it returns. A quote-aware
/// scan here would be a second parser living beside the real one and drifting from it, to decide
/// something no caller can observe.
fn first_command(line: &str) -> &str {
    let line = line.trim_start().trim_start_matches(SEPARATORS);
    match line.find(SEPARATORS) {
        Some(end) => &line[..end],
        None => line,
    }
}

/// The index of the first word after a wrapper's own options and assignments, or `None` when the
/// wrapper was the last word.
fn after_wrapper(words: &[&str], mut at: usize) -> Option<usize> {
    while at < words.len() {
        let word = words[at];
        if is_assignment(word) {
            at += 1;
            continue;
        }
        if word.starts_with('-') && word.len() > 1 {
            at += if VALUE_FLAGS.contains(&word) { 2 } else { 1 };
            continue;
        }
        return Some(at);
    }
    None
}

/// Whether a word is a `NAME=value` assignment.
fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Whether a word is shaped like a subcommand rather than an option, a path or a file name.
fn is_subcommand(word: &str) -> bool {
    word.starts_with(|c: char| c.is_ascii_alphabetic())
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// What may be stored for a line: the `argv` to keep, and its `head`.
///
/// Both are empty for a line with no command in it, which is the signal not to write a row at all.
/// When the line trips a filter the two come back equal — see the module note on why that is the
/// right failure.
pub fn prepare(line: &str) -> (String, String) {
    let line = line.trim();
    let head = head_of(line);
    if head.is_empty() {
        return (String::new(), String::new());
    }
    if is_risky(line) {
        return (head.clone(), head);
    }
    (line.to_string(), head)
}

/// Whether a line's arguments must not be written down.
///
/// None of these is load-bearing on its own, and none of them needs to be: a false positive costs
/// one line's worth of resolution in a statistics table.
pub fn is_risky(line: &str) -> bool {
    if line.len() > MAX_ARGV {
        return true;
    }
    // A heredoc body, or any continued line. Never recorded: the interesting text of a heredoc is
    // by definition not the command, and a multi-line entry can never be offered as a suggestion
    // anyway.
    if line.contains('\n') {
        return true;
    }

    let words: Vec<&str> = line.split_whitespace().collect();
    let head = head_of(line);
    let mut parts = head.split(' ');
    let command = parts.next().unwrap_or_default();
    let subcommand = parts.next().unwrap_or_default();
    if SECRET_COMMANDS.contains(&command) || SECRET_SUBCOMMANDS.contains(&(command, subcommand)) {
        return true;
    }

    for (at, word) in words.iter().enumerate() {
        if is_risky_word(word) {
            return true;
        }
        // `-u user:pass`, which curl and friends take and which no shape test would catch.
        if *word == "-u" && words.get(at + 1).is_some_and(|next| is_user_password(next)) {
            return true;
        }
    }
    false
}

/// Whether one word is, or plainly holds, a credential.
fn is_risky_word(word: &str) -> bool {
    if secret_option().is_match(word) {
        return true;
    }
    // A glued short option with a value — `-phunter2`. Long options are excluded because
    // `--prefix=/usr` is not a password.
    if word.len() > 2 && word.starts_with("-p") && !word.starts_with("--") {
        return true;
    }
    if word.len() >= "Authorization:".len() && word.to_ascii_lowercase().contains("authorization:")
    {
        return true;
    }
    if is_assignment(word) {
        let (name, value) = word.split_once('=').unwrap_or((word, ""));
        let name = name.to_ascii_uppercase();
        if SECRET_NAMES.iter().any(|secret| name.contains(secret)) && could_be_a_secret(value) {
            return true;
        }
    }
    is_known_key(word) || looks_like_a_key(word)
}

/// Whether an assignment's *value* could be the secret, rather than pointing at where one is.
///
/// **The name alone is not enough**, and a real line proved it:
///
/// ```text
/// sudo make hardware-smoke SIGN_KEY="$MOK/MOK.priv" SIGN_CERT="$MOK/MOK.der"
/// ```
///
/// `SIGN_KEY` contains `KEY`, so the whole line was reduced to `make` and never appeared in the
/// history again — while the value it was hiding was `$MOK/MOK.priv`, a path to a file. Three
/// shapes cannot be the secret and are the overwhelming majority of what people write:
///
/// ```text
/// KEY=            nothing to leak
/// KEY="$MOK/x"    a variable reference; whatever is secret lives in the variable, not here
/// KEY=~/x.priv    a path, which names a key rather than being one
/// ```
///
/// A literal — `KEY=hunter2`, `TOKEN=abc123` — still trips it, which is the case the rule is for.
fn could_be_a_secret(value: &str) -> bool {
    let value = value.trim_matches(['"', '\'']);
    if value.is_empty() || value.contains('$') {
        return false;
    }
    // A path, in any of the ways one is written. A base64 secret can hold a `/` too, which is why
    // this is not the last word on the matter: `is_known_key` and `looks_like_a_key` still read the
    // value afterwards, so a blob that happens to contain a slash is caught by its shape.
    let a_path = value.starts_with(['/', '~', '.']) || value.contains('/');
    !a_path
}

/// The credential formats that announce themselves.
fn is_known_key(word: &str) -> bool {
    let value = word.rsplit(['=', ':']).next().unwrap_or(word);
    for candidate in [word, value] {
        if candidate.starts_with("sk-")
            || candidate.starts_with("AKIA")
            || candidate.starts_with("eyJ")
            || candidate.starts_with("-----BEGIN")
        {
            return true;
        }
        if let Some(rest) = candidate.strip_prefix("gh")
            && rest.starts_with(['p', 'o', 'u', 's', 'r'])
            && rest[1..].starts_with('_')
        {
            return true;
        }
    }
    false
}

/// Whether a word has the shape of a key rather than of anything a person typed.
///
/// Long, mixed case, with digits, and with none of the punctuation that makes a path a path.
fn looks_like_a_key(word: &str) -> bool {
    word.len() >= 24
        && !word.contains('/')
        && !word.contains('.')
        && word.chars().any(|c| c.is_ascii_uppercase())
        && word.chars().any(|c| c.is_ascii_lowercase())
        && word.chars().any(|c| c.is_ascii_digit())
}

/// Whether a word is a `user:password` pair.
fn is_user_password(word: &str) -> bool {
    word.split_once(':')
        .is_some_and(|(user, password)| !user.is_empty() && !password.is_empty())
}

/// Directories that are excluded as themselves, not as the top of a subtree.
///
/// `/tmp` is a lobby. Nobody works *in* it, so recording it buys nothing, but plenty of people work
/// in `/tmp/build-xyz` and excluding those with it would silently delete the feature for them.
const EXCLUDED_DIRS: &[&str] = &["/tmp"];

/// Path components that exclude everything beneath them.
///
/// A store that ranks `~/.cargo/registry/src/index.crates.io-abcd/serde-1.0.219` because a build
/// touched it is worse than useless, and the same goes for the thousand directories a
/// `node_modules` is.
const EXCLUDED_COMPONENTS: &[&str] = &[".git", "node_modules"];

/// Whether a directory must be kept out of the store entirely — not merely out of `run`.
///
/// This is the design's directory exclusion list (`docs/research/smart-cd.md`, Privacy and size,
/// item 6) minus `$HOME`, which the directory queries refuse as a *jump target* while still
/// recording what was run there — see `super::query::Track::directories_ranked`. Note which of these
/// are subtrees and which are not — the design spells out "anything under" for the two components
/// and names the other as a directory, and the difference is the difference between excluding
/// `/tmp` and excluding everybody who builds in it.
pub fn is_excluded(path: &str) -> bool {
    let path = path.trim_end_matches('/');
    if EXCLUDED_DIRS.contains(&path) {
        return true;
    }
    path.split('/')
        .any(|component| EXCLUDED_COMPONENTS.contains(&component))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrapper_is_not_the_command() {
        assert_eq!(head_of("sudo cargo build --release"), "cargo build");
        assert_eq!(head_of("doas systemctl restart nginx"), "systemctl restart");
        assert_eq!(head_of("time make -j8"), "make");
        assert_eq!(head_of("nice -n 10 cargo test"), "cargo test");
        assert_eq!(head_of("sudo -u root cargo build"), "cargo build");
        assert_eq!(head_of("env FOO=bar git status"), "git status");
        // A wrapper with nothing after it is the command you ran.
        assert_eq!(head_of("time"), "time");
        assert_eq!(head_of("sudo -i"), "sudo");
    }

    #[test]
    fn a_leading_assignment_is_not_the_command() {
        assert_eq!(head_of("RUST_LOG=debug cargo test"), "cargo test");
        assert_eq!(head_of("A=1 B=2 sudo make install"), "make");
        // Not an assignment: an `=` inside an argument, or a name that is not a name.
        assert_eq!(head_of("./x=y arg"), "./x=y");
    }

    /// The column exists so that "what does cargo cost me" is answerable per activity. Grouping
    /// `cargo build` with `cargo test` would make it useless.
    #[test]
    fn a_subcommand_stays_with_its_tool() {
        assert_eq!(head_of("git commit -m 'x y'"), "git commit");
        assert_eq!(head_of("cargo run --example xyz"), "cargo run");
        // Only for tools that have subcommands, and only when it looks like one.
        assert_eq!(head_of("ls build"), "ls");
        assert_eq!(head_of("git -C /tmp status"), "git");
        assert_eq!(head_of("cargo --version"), "cargo");
        assert_eq!(head_of("cargo ./script"), "cargo");
    }

    /// A separator glued to the command word used to travel with it into the `head` column, so
    /// `cd; pwd` was filed under `cd;` and never counted with any other `cd`. Observed in a real
    /// session, not imagined.
    #[test]
    fn punctuation_does_not_become_part_of_the_tool_name() {
        assert_eq!(head_of("cd; pwd"), "cd");
        assert_eq!(head_of("make&"), "make");
        assert_eq!(head_of("ls|wc -l"), "ls");
        assert_eq!(head_of("cargo build && ./run"), "cargo build");
        assert_eq!(head_of("cargo test; echo done"), "cargo test");
        assert_eq!(head_of("cargo build > log.txt"), "cargo build");
        assert_eq!(head_of("grep x 2>/dev/null"), "grep");
        // An opening bracket must not leave an empty head, which would drop the row outright.
        assert_eq!(head_of("(cd build && make)"), "cd");
        // What follows the first command is somebody else's head, not part of this one.
        assert_eq!(head_of("sudo cargo build; rm -rf /"), "cargo build");
    }

    #[test]
    fn an_empty_line_has_no_head() {
        assert_eq!(head_of(""), "");
        assert_eq!(head_of("   "), "");
        assert_eq!(head_of("\n\ncargo build"), "");
    }

    /// The structural mitigation: the row survives, the arguments do not.
    #[test]
    fn a_risky_line_keeps_its_head_and_loses_its_arguments() {
        assert_eq!(
            prepare("curl --token abcdef https://x"),
            ("curl".to_string(), "curl".to_string())
        );
        // And an ordinary line is kept whole.
        assert_eq!(
            prepare("  cargo run --example xyz  "),
            (
                "cargo run --example xyz".to_string(),
                "cargo run".to_string()
            )
        );
        // Nothing to record at all.
        assert_eq!(prepare("   "), (String::new(), String::new()));
    }

    #[test]
    fn credentials_are_recognised_in_every_shape_they_arrive_in() {
        for line in [
            "gpg --decrypt secrets.asc",
            "gh auth login",
            "docker login -u me registry.example.com",
            "mysql -h db -p",
            "curl --password hunter2 https://x",
            "curl -H 'Authorization: Bearer x' https://x",
            "psql postgres://x",
            "export GITHUB_TOKEN=abc",
            "aws configure set x",
            "curl -u alice:hunter2 https://x",
            "mysqldump -phunter2 db",
            "echo ghp_A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6",
            "openai --api-key sk-abcdefghijklmnop",
            "aws s3 ls --profile AKIAIOSFODNN7EXAMPLE",
            "echo eyJhbGciOiJIUzI1NiJ9.x",
            "deploy --key aB3dE5fG7hJ9kL1mN3pQ5rS7",
        ] {
            assert!(is_risky(line), "{line} should not be recorded in full");
        }
    }

    #[test]
    fn ordinary_lines_are_recorded_in_full() {
        for line in [
            "cargo run --example xyz",
            "git commit -m 'fix the parser'",
            "ls -la /var/log",
            "make --prefix=/usr install",
            "git log --author=me",
            "rg 'fn main' src/",
            "docker compose up -d",
        ] {
            assert!(!is_risky(line), "{line} is not a secret");
        }
    }

    /// **A name that says `KEY` beside a value that is a path.** Reported from a real session: this
    /// line went missing from the history every time it was run, while the prediction model — which
    /// learns from a different table — went on suggesting it, so the shell looked like it was
    /// forgetting on purpose.
    #[test]
    fn an_assignment_pointing_at_a_key_file_is_not_a_key() {
        let line = concat!(
            r#"sudo make hardware-smoke HARDWARE_SERIALS="$SERIALS" SIGN_FILE="$SIGN" "#,
            r#"SIGN_HASH=sha512 SIGN_KEY="$MOK/MOK.priv" SIGN_CERT="$MOK/MOK.der""#
        );
        assert!(!is_risky(line), "the values are paths and variables");
        assert_eq!(prepare(line).0, line, "recorded whole");

        for kept in [
            "make SIGN_KEY=/etc/keys/MOK.priv",
            "make SIGN_KEY=~/keys/MOK.priv",
            "make SIGN_KEY=./MOK.priv",
            "deploy TOKEN=$CI_TOKEN",
            "deploy API_KEY=",
        ] {
            assert!(!is_risky(kept), "{kept} points at a secret, it is not one");
        }
    }

    /// And the case the rule is actually for still trips: a literal value beside a telling name.
    #[test]
    fn a_literal_beside_a_secret_name_is_still_dropped() {
        for line in [
            "deploy API_KEY=abc123",
            "run PASSWORD=hunter2",
            "make SIGN_KEY=s3cr3t",
        ] {
            assert!(is_risky(line), "{line} carries the value itself");
        }
    }

    /// **An assignment in front of a command is not a secret by itself.** It used to be: any line
    /// starting `FOO=bar` lost its arguments, so `WGPU_BACKEND=vulkan rerun` and
    /// `RUST_LOG=debug cargo test` were both stored as bare command names. The credential case the
    /// rule existed for is caught by the value, wherever in the line it sits.
    #[test]
    fn a_leading_assignment_is_read_like_any_other_word() {
        assert!(!is_risky("WGPU_BACKEND=vulkan rerun"));
        assert!(!is_risky("RUST_LOG=debug cargo test"));
        assert!(!is_risky("A=1 B=2 make install"));

        assert!(is_risky(
            "GITHUB_TOKEN=ghp_A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6 gh pr list"
        ));
        assert!(is_risky("GITHUB_TOKEN=hunter2 gh pr list"));
        assert!(is_risky("OPENAI_API_KEY=sk-abcdefghijklmnop chat"));
    }

    /// The glued `-p<value>` rule cannot tell `-phunter2` from `-print`, and it is not supposed to
    /// try: a filter that has to be perfect is a bad filter. This pins what the false positive
    /// costs — one line's arguments, never the row — so that nobody later "fixes" it by narrowing
    /// the rule until a real password gets through.
    #[test]
    fn a_false_positive_costs_arguments_and_nothing_else() {
        assert!(is_risky("find . -print"));
        assert_eq!(
            prepare("find . -print"),
            ("find".to_string(), "find".to_string())
        );
    }

    /// A paste is not a habit, and a continued line is not a suggestion.
    #[test]
    fn a_paste_and_a_heredoc_body_are_never_recorded_whole() {
        assert!(is_risky(&format!("echo {}", "x".repeat(MAX_ARGV))));
        assert!(is_risky("cat <<EOF\nsome body\nEOF"));
        assert_eq!(prepare("cat <<EOF\nbody\nEOF").0, "cat");
    }

    #[test]
    fn whole_subtrees_stay_out_of_the_store() {
        assert!(is_excluded("/w/p/node_modules/react/lib"));
        assert!(is_excluded("/w/p/.git/refs"));
        assert!(!is_excluded("/w/p/src"));
        assert!(
            !is_excluded("/w/p/gitignore"),
            "a component is matched whole, not as a prefix"
        );
    }

    /// `$HOME` is deliberately *not* on the exclusion list, though the design puts it there.
    ///
    /// Excluding it at write time throws away every command run at home, which kills the
    /// directory-aware suggestion in the one directory many people spend the day in. Keeping it out
    /// of *jump candidates* is the part that was actually wanted, and that is enforced in the
    /// directory queries — see `query::tests::home_is_remembered_but_never_jumped_to`.
    #[test]
    fn home_itself_is_still_written_down() {
        assert!(!is_excluded("/home/u"));
        assert!(!is_excluded("/home/u/src"));
    }

    /// `/tmp` is on the design's exclusion list as a *directory*, next to two entries that say
    /// "anything under". Nobody works in the lobby, and plenty of people work in a build tree
    /// under it — excluding those too would kill the feature for them with no diagnostic at all,
    /// which is the failure this store can least afford, since its only symptom is a `cd` that
    /// quietly never learns anything.
    #[test]
    fn a_build_tree_under_tmp_is_ordinary_work() {
        assert!(is_excluded("/tmp"));
        assert!(is_excluded("/tmp/"), "with or without the slash");
        assert!(!is_excluded("/tmp/build-xyz"));
        assert!(!is_excluded("/tmp/scratch/src"));
        assert!(!is_excluded("/tmpfoo"), "a prefix is not a component");
        // And the component rules still reach inside it.
        assert!(is_excluded("/tmp/p/node_modules/react"));
    }
}
