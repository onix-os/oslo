//! What goes in the columns after the label.
//!
//! A completion row is a label, a kind badge, and then any number of *info columns*. The first is
//! the description; what follows is whatever the kind has left to say — a file's size, a
//! directory's entry count, what an alias expands to. A config can replace the lot from Lua.
//!
//! **Everything here runs at render time, on the visible rows only.** That is not an optimisation
//! detail, it is the reason a size column is affordable at all: `ls /usr/bin/<Tab>` offers three
//! thousand candidates, and a `stat` per candidate would be three thousand syscalls per keystroke
//! for a listing that shows fifteen rows. Fifteen `stat`s per frame is nothing. So [`facts_for`]
//! is called from the render loop after the visible slice is chosen, and never before.

use super::CompletionCandidate;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::SystemTime;

/// How many entries a directory is counted up to before the count is reported as "and more".
///
/// A count is one `read_dir`, which is cheap for a source tree and emphatically not cheap for a
/// spool directory with half a million files in it — and that read would happen on every frame,
/// while the user is holding an arrow key. Past the cap the answer is `999+`, which tells you the
/// same thing the true number would: it is big.
const MAX_COUNTED_ENTRIES: usize = 1000;

/// What can be learned about a candidate without asking the shell.
///
/// Every field is optional because every one of them can fail: the path may have been deleted
/// between the listing and the draw, or be unreadable, or not be a path at all.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub size: Option<u64>,
    pub entries: Option<usize>,
    /// Whether [`Facts::entries`] stopped at the count cap rather than reaching the end.
    pub entries_capped: bool,
    pub mtime: Option<SystemTime>,
    pub mode: Option<u32>,
    pub is_dir: bool,
    pub is_symlink: bool,
}

/// Learn what can be learned about one candidate. One `stat`, plus a capped `read_dir` for a
/// directory. Call this for the rows being drawn and no others.
pub fn facts_for(cand: &CompletionCandidate) -> Facts {
    let mut facts = Facts::default();
    let Some(path) = cand.path.as_deref() else {
        return facts;
    };
    // `symlink_metadata` first so a broken link is still describable; the follow-through below is
    // what decides whether it behaves as a directory.
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        facts.is_symlink = meta.file_type().is_symlink();
        facts.mtime = meta.modified().ok();
        facts.mode = Some(std::os::unix::fs::MetadataExt::mode(&meta));
        facts.size = Some(meta.len());
        facts.is_dir = meta.is_dir();
    }
    if facts.is_symlink
        && let Ok(meta) = std::fs::metadata(path)
    {
        facts.is_dir = meta.is_dir();
        facts.size = Some(meta.len());
    }
    if facts.is_dir {
        let mut count = 0usize;
        if let Ok(entries) = std::fs::read_dir(path) {
            for _ in entries {
                count += 1;
                if count >= MAX_COUNTED_ENTRIES {
                    facts.entries_capped = true;
                    break;
                }
            }
        }
        facts.entries = Some(count);
    }
    facts
}

/// A byte count as something that fits in five cells: `4.2K`, `918B`, `1.3M`.
///
/// Binary units, because a shell reports what the filesystem reports and that is what `ls -lh`
/// says. One decimal only below ten, so the column never rags.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{}B", bytes)
    } else if value < 10.0 {
        format!("{:.1}{}", value, UNITS[unit])
    } else {
        format!("{:.0}{}", value, UNITS[unit])
    }
}

/// How long ago, in the shortest form that is still true: `3s`, `2h`, `5d`, `1y`.
pub fn human_age(mtime: SystemTime) -> String {
    let Ok(elapsed) = SystemTime::now().duration_since(mtime) else {
        // In the future — a clock skew or a freshly touched file. Saying `0s` is closer to the
        // truth than a wrapped-around number would be.
        return "0s".to_string();
    };
    let secs = elapsed.as_secs();
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86399 => format!("{}h", secs / 3600),
        86400..=2591999 => format!("{}d", secs / 86400),
        2592000..=31535999 => format!("{}mo", secs / 2592000),
        _ => format!("{}y", secs / 31536000),
    }
}

/// The permission bits as `rwxr-xr-x`.
pub fn human_mode(mode: u32) -> String {
    let mut out = String::with_capacity(9);
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 0o7;
        out.push(if bits & 0o4 != 0 { 'r' } else { '-' });
        out.push(if bits & 0o2 != 0 { 'w' } else { '-' });
        out.push(if bits & 0o1 != 0 { 'x' } else { '-' });
    }
    out
}

/// The default info columns for a candidate: its description, then whatever its kind adds.
///
/// The second column exists only where the kind leaves something unsaid. A directory's name says
/// it is a directory and the badge says so again, so what is worth adding is how much is *in* it;
/// an alias's name says nothing at all about what it runs, so it gets its expansion.
///
/// **A candidate with no description does not hold its place.** Nothing has a description *and* a
/// count, so a menu mixing the two — `cd <Tab>`, where the spec's `-` and `~` are described and the
/// directories are counted — put the counts in the second column and left the first one blank
/// under the descriptions:
///
/// ```text
///   -           value   Switch to the last used folder
///   config/     dir                                     2 items
/// ```
///
/// Dropping the empty description lets the count move into the column the description would have
/// used, which is the rule `oslo.completion.descriptions = false` already follows for the whole
/// menu — see the note in `render`.
pub fn builtin_columns(cand: &CompletionCandidate, facts: &Facts) -> Vec<String> {
    let description = cand.description.clone().unwrap_or_default();
    let extra = match cand.kind.as_deref() {
        Some("dir") | Some("directory") => match (facts.entries, facts.entries_capped) {
            (Some(_), true) => format!("{}+ items", MAX_COUNTED_ENTRIES - 1),
            (Some(1), false) => "1 item".to_string(),
            (Some(n), false) => format!("{n} items"),
            (None, _) => String::new(),
        },
        Some("file") => facts.size.map(human_size).unwrap_or_default(),
        // Where the program actually is, which is what `type` would tell you and what the name
        // alone cannot: `ls` in `~/bin` shadowing `/usr/bin/ls` looks identical in the listing
        // until this column says otherwise.
        Some("command") => where_is(&cand.display).unwrap_or_default(),
        // The one place the completer knew something the renderer could not work out for itself.
        Some("alias") => cand.detail.clone().unwrap_or_default(),
        _ => String::new(),
    };
    match (description.is_empty(), extra.is_empty()) {
        (true, true) => Vec::new(),
        (true, false) => vec![extra],
        _ => vec![description, extra],
    }
}

/// The directory the first `$PATH` match for `name` lives in.
///
/// Walked here rather than carried on the candidate because the command index keeps only names —
/// and walking is affordable *because this runs for visible rows only*. Fifteen rows against a
/// ten-entry `$PATH` is a hundred and fifty `stat`s a frame; doing it when the candidates were
/// collected would be that many times three thousand.
fn where_is(name: &str) -> Option<String> {
    let path = std::env::var("PATH").ok()?;
    for dir in path.split(':').filter(|d| !d.is_empty()) {
        let candidate = std::path::Path::new(dir).join(name);
        if nix::unistd::access(&candidate, nix::unistd::AccessFlags::X_OK).is_ok() {
            return Some(dir.to_string());
        }
    }
    None
}

/// A hook that decides a candidate's columns, replacing [`builtin_columns`] entirely.
///
/// Returning `None` falls back to the built-ins for that candidate, so a Lua function can answer
/// for the kinds it cares about and leave the rest alone.
pub type Provider = Rc<dyn Fn(&CompletionCandidate, &Facts) -> Option<Vec<String>>>;

thread_local! {
    /// Thread-local rather than a global: the provider calls into the Lua interpreter, which is
    /// not `Send`, and only the thread running the editor ever draws a dropdown.
    static PROVIDER: RefCell<Option<Provider>> = const { RefCell::new(None) };
}

/// Install the hook `oslo.completion.columns` describes. Passing `None` restores the built-ins.
pub fn set_provider(provider: Option<Provider>) {
    PROVIDER.with(|slot| *slot.borrow_mut() = provider);
}

/// The columns to draw for one candidate: the config's answer, or the built-in one.
pub fn columns_for(cand: &CompletionCandidate, facts: &Facts) -> Vec<String> {
    columns_and_source(cand, facts).0
}

/// As [`columns_for`], and whether the answer came from a config rather than the built-ins.
///
/// The caller needs to know which, because `oslo.completion.descriptions = false` is about *the
/// description* — the first built-in column. A config that has defined its own columns has taken
/// the question over, and blanking whatever it happened to put first would be the shell second-
/// guessing it.
fn columns_and_source(cand: &CompletionCandidate, facts: &Facts) -> (Vec<String>, bool) {
    let from_config = PROVIDER.with(|slot| {
        // Cloned out of the cell before the call: the hook runs Lua, and Lua can complete another
        // word, which would come back through here and panic on the outstanding borrow.
        let provider = slot.borrow().clone();
        provider.and_then(|p| p(cand, facts))
    });
    match from_config {
        Some(columns) => (columns, true),
        None => (builtin_columns(cand, facts), false),
    }
}

/// The columns for a whole visible slice, squared off so every row has the same count.
///
/// A ragged list would let one row's second column line up under another's third.
///
/// **A column no row fills is removed, wherever it sits.** Not just the trailing ones: a listing of
/// plain files has nothing to say in the description column, and leaving it there as a hole cost
/// twenty-five cells that the badge and the size column then had to be dropped to pay for. On an
/// eighty-column terminal with a deep prompt that is the difference between a row that says
/// `examples/  dir  12 items` and one that says `examples/` followed by blanks.
pub fn columns_for_rows(candidates: &[CompletionCandidate]) -> Vec<Vec<String>> {
    with_descriptions(
        candidates,
        crate::settings::current().completion.descriptions,
    )
}

/// A command's one-line description, from its spec, if it has one and does not already carry it.
///
/// Borrowed unless there is something to add, so a listing of files costs no allocation at all.
/// Only the kinds that *are* a command name are looked up: a filename is not a spec name, and
/// asking for one is the probe this whole seam exists to avoid.
fn described_by_spec(cand: &CompletionCandidate) -> std::borrow::Cow<'_, CompletionCandidate> {
    use std::borrow::Cow;
    if cand.description.is_some() {
        return Cow::Borrowed(cand);
    }
    let named = matches!(
        cand.kind.as_deref(),
        Some("command") | Some("builtin") | Some("function") | Some("tool")
    );
    if !named {
        return Cow::Borrowed(cand);
    }
    match crate::spec::custom::find(&cand.display) {
        Some(spec) if !spec.description.is_empty() => {
            let mut owned = cand.clone();
            owned.description = Some(spec.description.to_string());
            Cow::Owned(owned)
        }
        _ => Cow::Borrowed(cand),
    }
}

/// [`columns_for_rows`], told whether descriptions are on rather than reading it.
///
/// **The setting is process-wide, and reading it here made this untestable without writing it.**
/// The test below did exactly that, and every other test rendering at the same moment read what it
/// had installed: `long_labels_and_descriptions_are_ellipsised` failed about once in ten full runs
/// because it happened to render while descriptions were off, in a listing it never asked for.
/// Taking the answer as an argument leaves the shell's own startup as the only writer of that
/// global, which is the only way a test suite that runs in parallel can share one.
pub(crate) fn with_descriptions(
    candidates: &[CompletionCandidate],
    descriptions: bool,
) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = candidates
        .iter()
        .map(|c| {
            // Asked here, for a row being drawn, rather than for every candidate the completer
            // produced — see `Completer::command_candidate`. Skipped entirely when descriptions
            // are off, which is the case the eager version could not skip at all.
            let c = match descriptions {
                true => described_by_spec(c),
                false => std::borrow::Cow::Borrowed(c),
            };
            let (mut columns, from_config) = columns_and_source(&c, &facts_for(&c));
            // The description is the first built-in column, so turning descriptions off removes
            // it and lets a size or an item count move left into the space — rather than leaving
            // a twenty-five-cell gutter where the description used to be.
            if !descriptions && !from_config && !columns.is_empty() {
                columns.remove(0);
            }
            columns
        })
        .collect();
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    for row in &mut rows {
        row.resize(width, String::new());
    }
    let filled: Vec<bool> = (0..width)
        .map(|i| rows.iter().any(|r| !r[i].is_empty()))
        .collect();
    for row in &mut rows {
        let mut keep = filled.iter();
        row.retain(|_| *keep.next().unwrap_or(&false));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(kind: &str) -> CompletionCandidate {
        let mut c = CompletionCandidate::new("x".into(), "x".into(), None);
        c.kind = Some(kind.to_string());
        c
    }

    /// What the *kind* adds, isolated from whether the candidate is described: a description is
    /// given so the answer stays in the second column either way.
    fn kind_column(mut cand: CompletionCandidate, facts: &Facts) -> String {
        cand.description = Some("d".into());
        builtin_columns(&cand, facts)[1].clone()
    }

    #[test]
    fn sizes_read_the_way_ls_h_reads() {
        assert_eq!(human_size(0), "0B");
        assert_eq!(human_size(918), "918B");
        assert_eq!(human_size(999), "999B");
        assert_eq!(human_size(1024), "1.0K");
        assert_eq!(human_size(1536), "1.5K");
        assert_eq!(human_size(4300), "4.2K");
        assert_eq!(human_size(1024 * 1024 * 3 / 2), "1.5M");
        // Past ten the decimal goes, so the column never needs a sixth cell.
        assert_eq!(human_size(1024 * 1024 * 42), "42M");
    }

    #[test]
    fn modes_read_as_permission_bits() {
        assert_eq!(human_mode(0o755), "rwxr-xr-x");
        assert_eq!(human_mode(0o100644), "rw-r--r--");
        assert_eq!(human_mode(0), "---------");
    }

    #[test]
    fn ages_shorten_as_they_grow() {
        let ago = |secs| human_age(SystemTime::now() - std::time::Duration::from_secs(secs));
        assert_eq!(ago(5), "5s");
        assert_eq!(ago(120), "2m");
        assert_eq!(ago(7200), "2h");
        assert_eq!(ago(86400 * 3), "3d");
        assert_eq!(ago(86400 * 60), "2mo");
        assert_eq!(ago(86400 * 800), "2y");
    }

    /// A command says where it lives, which is the one thing its name cannot: two `ls` entries in
    /// a listing are indistinguishable until the column names the directory.
    #[test]
    fn a_command_says_which_directory_it_came_from() {
        let mut sh = cand("command");
        sh.display = "sh".to_string();
        let column = kind_column(sh, &Facts::default());
        assert!(
            column.starts_with('/'),
            "a command on $PATH names its directory, got {column:?}"
        );

        // Something that is not on `$PATH` says nothing rather than guessing.
        let mut absent = cand("command");
        absent.display = "definitely-not-a-program-anywhere".to_string();
        assert_eq!(kind_column(absent, &Facts::default()), "");
    }

    /// A kind whose name already tells the whole story adds nothing; a kind that leaves something
    /// unsaid gets a column for it.
    #[test]
    fn only_kinds_with_something_left_to_say_get_a_second_column() {
        let facts = Facts {
            size: Some(4300),
            entries: Some(12),
            ..Facts::default()
        };
        assert_eq!(kind_column(cand("file"), &facts), "4.2K");
        assert_eq!(kind_column(cand("dir"), &facts), "12 items");
        assert_eq!(kind_column(cand("builtin"), &facts), "");

        let mut alias = cand("alias");
        alias.detail = Some("git status --short".to_string());
        assert_eq!(kind_column(alias, &facts), "git status --short");
    }

    /// A huge directory is not counted to the end, and says so rather than lying with a round
    /// number — the read would otherwise repeat on every frame while an arrow key is held.
    #[test]
    fn an_uncounted_directory_says_so() {
        let facts = Facts {
            entries: Some(MAX_COUNTED_ENTRIES),
            entries_capped: true,
            ..Facts::default()
        };
        assert_eq!(kind_column(cand("dir"), &facts), "999+ items");
        // Singular reads as English rather than "1 items".
        let one = Facts {
            entries: Some(1),
            ..Facts::default()
        };
        assert_eq!(kind_column(cand("dir"), &one), "1 item");
    }

    /// A column that is empty on every row is not drawn as a gutter of spaces.
    #[test]
    fn a_column_no_row_fills_is_dropped_whole() {
        let rows = columns_for_rows(&[cand("builtin"), cand("builtin")]);
        assert!(rows.iter().all(Vec::is_empty), "{rows:?}");

        // One row filling it is enough to keep it for all of them.
        let mut alias = cand("alias");
        alias.detail = Some("ls -la".to_string());
        let rows = columns_for_rows(&[cand("builtin"), alias]);
        // The description column went — nothing had one — and the expansion column stayed.
        assert_eq!(rows[0], vec!["".to_string()]);
        assert_eq!(rows[1], vec!["ls -la".to_string()]);
    }

    /// The one that cost the badge and the size column on a real terminal: a listing of plain
    /// files has no descriptions, and an empty description column in front of the size is not a
    /// blank gutter to be paid for — it is a column that should not exist.
    #[test]
    fn an_empty_column_in_the_middle_goes_too_not_just_a_trailing_one() {
        let mut file = cand("file");
        file.description = None;
        let mut described = cand("alias");
        described.description = None;
        described.detail = Some("git status".to_string());
        let rows = columns_for_rows(&[file, described]);
        assert_eq!(rows[0].len(), 1, "{rows:?}");
        assert_eq!(rows[1], vec!["git status".to_string()]);
    }

    /// `descriptions = false` removes the description and lets the rest move left. It must not
    /// take the size column with it, and it must not leave a gutter where the description was.
    #[test]
    fn turning_descriptions_off_drops_only_the_description() {
        let mut c = cand("alias");
        c.description = Some("run it".to_string());
        c.detail = Some("git status".to_string());

        // Asked directly rather than through the installed settings. Writing those from a test
        // reaches every other test in the process; see `with_descriptions`.
        let off = with_descriptions(std::slice::from_ref(&c), false);
        let on = with_descriptions(std::slice::from_ref(&c), true);

        assert_eq!(off, vec![vec!["git status".to_string()]]);
        assert_eq!(
            on,
            vec![vec!["run it".to_string(), "git status".to_string()]]
        );
    }

    #[test]
    fn a_provider_may_answer_for_some_kinds_and_defer_on_others() {
        set_provider(Some(Rc::new(|c: &CompletionCandidate, _: &Facts| {
            (c.kind.as_deref() == Some("file")).then(|| vec!["mine".to_string()])
        })));
        assert_eq!(columns_for(&cand("file"), &Facts::default()), vec!["mine"]);
        assert_eq!(
            columns_for(&cand("dir"), &Facts::default()),
            builtin_columns(&cand("dir"), &Facts::default())
        );
        set_provider(None);
        assert_eq!(
            columns_for(&cand("file"), &Facts::default()),
            builtin_columns(&cand("file"), &Facts::default())
        );
    }

    #[test]
    fn an_undescribed_candidate_gives_its_column_up() {
        let counted = Facts {
            entries: Some(2),
            is_dir: true,
            ..Facts::default()
        };
        // `cd <Tab>` mixes the two: the spec's `-` is described, a directory is counted, and
        // neither has both. The count must start where the description starts, not one column
        // past it.
        assert_eq!(builtin_columns(&cand("dir"), &counted), ["2 items"]);

        let mut described = cand("dir");
        described.description = Some("the source".into());
        assert_eq!(
            builtin_columns(&described, &counted),
            ["the source", "2 items"],
            "a candidate with both still spends two columns"
        );

        assert!(
            builtin_columns(&cand("value"), &Facts::default()).is_empty(),
            "and one with neither spends none"
        );
    }
}

/// **The description is resolved for a drawn row, and not for anything else.**
///
/// The completer used to ask the spec loader for every matching `$PATH` name — thousands of probes
/// on the first bare Tab, each trying three directories by two extensions and fully parsing any
/// hit — to fill a column only the visible rows show. The lookup moved here; these two are what
/// that has to keep true, and what it now skips.
#[cfg(test)]
mod description_tests {
    use super::*;
    use crate::spec::CommandSpec;
    use crate::spec::custom::{forget, register};

    fn described(name: &str, description: &str) -> CommandSpec {
        CommandSpec {
            name: name.to_string(),
            description: description.to_string(),
            ..CommandSpec::default()
        }
    }

    fn row(kind: &str, descriptions: bool) -> Vec<String> {
        let mut cand = CompletionCandidate::new("gitish".into(), "gitish".into(), None);
        cand.kind = Some(kind.to_string());
        with_descriptions(&[cand], descriptions).remove(0)
    }

    #[test]
    fn a_drawn_command_row_gets_its_spec_description() {
        forget();
        register(described("gitish", "a version control thing"));

        let drawn = row("command", true);
        assert!(
            drawn.iter().any(|c| c == "a version control thing"),
            "the column arrives for a row being drawn: {drawn:?}"
        );

        // Off, it is not looked up at all — the case the eager version could not skip, because it
        // had already paid for the lookup by the time the setting was read.
        let bare = row("command", false);
        assert!(
            !bare.iter().any(|c| c == "a version control thing"),
            "and is skipped when descriptions are off: {bare:?}"
        );

        // A filename is not a spec name, so that kind is never asked about at all.
        let file = row("file", true);
        assert!(
            !file.iter().any(|c| c == "a version control thing"),
            "and a file is not looked up: {file:?}"
        );
        forget();
    }
}
