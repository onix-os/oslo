//! Applying what the config set.
//!
//! The theme and the settings are *read out of the interpreter* once the config file has run,
//! rather than pushed in as it runs. A config is a script — it can set a colour, compute another,
//! and change its mind — and reading the final state once is the only way to see what it decided
//! rather than what it passed through on the way.

use crate::lua::LuaEngine;

/// Read `oslo.theme` and the rest of the settings out of the interpreter and install them.
///
/// Complaints are printed rather than swallowed: an element that quietly keeps its default looks
/// exactly like oslo ignoring the config, which is the hardest kind of mistake to find.
/// Say one config problem, with a caret into the file that caused it when it can be found.
///
/// **The value is looked for rather than tracked.** A problem reads `oslo.scratch.key: 'ctrl-nope'
/// is not a key name`, and by the time it is reported the config has already *run* — there is no
/// span left anywhere, and threading one out of a Lua interpreter to report a string it handed back
/// would be a large change for a small message.
///
/// So the quoted value is pulled out of the problem and looked for in the file, which is right
/// whenever the config wrote it literally — which is nearly always, because a config that computed
/// the value would not have got it wrong in a way worth pointing at. Where it is not found, nothing
/// is drawn and the one-liner is what it always was.
fn say(problem: &str, files: &[std::path::PathBuf]) {
    let message = format!("oslo: {problem}");
    if oslo_base::diag::enabled()
        && let Some(value) = quoted(problem)
    {
        for path in files {
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            let Some(at) = text.find(value) else {
                continue;
            };
            let report = oslo_base::diag::Report {
                message: &message,
                source: &path.display().to_string(),
                label: "not one of the names this setting takes",
                help: None,
            };
            if oslo_base::diag::draw_source(&text, at..at + value.len(), &report) {
                return;
            }
        }
    }
    eprintln!("{message}");
}

/// The `'…'` a problem quotes, which is the value the config wrote.
fn quoted(problem: &str) -> Option<&str> {
    let (_, rest) = problem.split_once('\'')?;
    let (value, _) = rest.split_once('\'')?;
    (!value.is_empty()).then_some(value)
}

pub fn apply(lua: &LuaEngine, files: &[std::path::PathBuf]) {
    let (theme, problems) = lua.read_theme();
    for problem in problems {
        say(&problem, files);
    }
    oslo_ui::theme::install(theme);

    let (settings, problems) = lua.read_settings();
    for problem in problems {
        say(&problem, files);
    }
    // `@name` is read on the expansion path, so the table is handed over rather than looked up:
    // a word expansion must not reach into the settings to find out what `@work` means.
    oslo_shell::expand::sugar::set_named_dirs(
        settings
            .dirs
            .iter()
            .map(|(name, path)| (name.clone(), path.clone()))
            .collect(),
    );

    // Installed rather than kept in the settings and consulted on each keystroke: the abbreviation
    // table is also written by the `abbr` builtin, and having two sources that disagree is worse
    // than having one that a config seeds. Cleared first so a reload does not leave behind an entry
    // the config no longer defines — but only when the config defines any, so a shell whose
    // abbreviations all come from `abbr` at the prompt does not lose them to a config reload.
    if !settings.abbr.is_empty() {
        oslo_ui::abbr::clear();
        for (name, expansion, anywhere) in &settings.abbr {
            let placement = if *anywhere {
                oslo_ui::abbr::Placement::Anywhere
            } else {
                oslo_ui::abbr::Placement::Command
            };
            oslo_ui::abbr::add(name, expansion, placement);
        }
    }

    oslo_ui::settings::install(settings);

    // **Installed rather than read**, because the block may be drawn by a program and the editor
    // cannot run one. A config that names none installs none and the rule is drawn instead.
    super::transcript::install(&lua.oslo_table());

    // Installed rather than read: unlike a theme, this one is a *function*, and it has to be
    // called once per visible row on every frame the dropdown draws.
    lua.install_column_provider();
    lua.install_command_completer();
}

#[cfg(test)]
mod tests {
    use super::quoted;

    /// The value a problem quotes is what the caret goes under, so pulling it out has to be exact.
    #[test]
    fn the_quoted_value_is_found() {
        assert_eq!(
            quoted("oslo.scratch.key: 'ctrl-nope' is not a key name"),
            Some("ctrl-nope")
        );
        assert_eq!(
            quoted("oslo.completion.sort: 'alphabetic' is not an order; use 'frecency' or 'alpha'"),
            Some("alphabetic"),
            "the first pair, not the ones in the advice"
        );
    }

    /// A problem with no value in it draws nothing, which is the answer that means "print the
    /// one-liner" — not a caret under an empty span somewhere arbitrary.
    #[test]
    fn a_problem_without_a_value_points_at_nothing() {
        assert_eq!(quoted("oslo.table: expected a table"), None);
        assert_eq!(quoted("nothing quoted here"), None);
        assert_eq!(quoted("an empty '' pair"), None);
        assert_eq!(quoted("one ' unbalanced quote"), None);
    }
}
