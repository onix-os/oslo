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
pub fn apply(lua: &LuaEngine) {
    let (theme, problems) = lua.read_theme();
    for problem in problems {
        eprintln!("oslo: {problem}");
    }
    oslo_ui::theme::install(theme);

    let (settings, problems) = lua.read_settings();
    for problem in problems {
        eprintln!("oslo: {problem}");
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

    // Installed rather than read: unlike a theme, this one is a *function*, and it has to be
    // called once per visible row on every frame the dropdown draws.
    lua.install_column_provider();
    lua.install_command_completer();
}
