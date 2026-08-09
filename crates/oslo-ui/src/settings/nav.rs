//! `oslo.builtin.nav` — how the filesystem navigator looks and how it moves.
//!
//! Split from the rest of the settings because it is the one group that describes a *widget*
//! rather than the editor: what a row is marked with, and whether typing a name walks into it.
//! Everything here is read once, when `nav` opens.

/// `oslo.builtin.nav` — presentation and navigation defaults for `nav`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nav {
    pub fullscreen: bool,
    pub position: crate::ask::chrome::Place,
    pub width: usize,
    pub border: crate::ask::Border,
    pub border_fg: Option<crate::theme::Color>,
    pub border_fit: crate::ask::chrome::Fit,
    pub legend: bool,
    pub legend_gap: usize,
    pub padding_x: usize,
    pub padding_y: usize,
    /// Zero uses the middle half of a full screen or up to fourteen inline rows.
    pub height: usize,
    pub hidden: bool,
    pub filter_at: crate::ask::Where,
    pub reverse: bool,
    pub scanner: bool,
    /// What is drawn in front of each name.
    pub icons: Icons,
    /// Whether typing a name walks into it once nothing else matches.
    pub type_nav: TypeNav,
}

/// `oslo.builtin.nav.type_nav` — filtering down to one directory enters it.
///
/// ```lua
/// oslo.builtin.nav.type_nav = { enabled = true, settle_ms = 500 }
/// ```
///
/// Typing `fuz` in a directory whose only match is `fuzz/` walks in without waiting for Enter,
/// which is what makes the filter feel like a path being typed rather than a search being run.
///
/// **Directories only.** A single matching *file* is left alone: opening it would mean choosing a
/// program to open it with, and being wrong about that is a far worse outcome than a keystroke.
///
/// # Why there is a deadline
///
/// The word is usually longer than the prefix that identifies it. `fuzz` stops being ambiguous at
/// `fuz`, so the entry happens with a `z` still to come — and that `z` would land in the *new*
/// directory's filter, which is a directory the person has not looked at yet. So for `settle_ms`
/// after an automatic entry, typed characters are dropped: the tail of the word you were already
/// typing cannot become the head of a search you did not ask for.
///
/// Only characters are dropped, and nothing else — Escape, the arrows, Enter and Backspace all
/// work through it, so the deadline can never make the widget feel stuck.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeNav {
    pub enabled: bool,
    /// How long typed characters are ignored after walking in. Zero drops nothing.
    pub settle: std::time::Duration,
}

impl Default for TypeNav {
    fn default() -> Self {
        TypeNav {
            enabled: true,
            // Long enough to swallow the rest of a word — a fast typist is ~150ms between keys, so
            // this covers three more of them — and short enough that it is over before anyone has
            // read the directory they just landed in and decided what to type next. Seconds would
            // be safer against the leaked keystroke and would read as a shell that had frozen.
            settle: std::time::Duration::from_millis(500),
        }
    }
}

/// `oslo.builtin.nav.icons` — the mark in front of a name, and what it is for a given extension.
///
/// ```lua
/// oslo.builtin.nav.icons = {
///   dir = "■", file = "≡",
///   ext = { <extension> = <mark>, ... },
/// }
/// ```
///
/// **Two marks are built in and no more.** `ext` starts empty: which mark a `.rs` or a `.md`
/// deserves is a matter of taste, of the font you run, and of what you actually work on — so it
/// belongs in a config file, not in this file. Nothing here should have to be edited to change how
/// a listing looks.
///
/// **One cell wide, or the column stops being one.** The marks are right-aligned as a block with
/// the rest of the row measured against them, and an emoji is two cells — so a listing where one
/// row is marked with an emoji and the others are not has that row's name starting a column over
/// from everybody else's.
///
/// This replaced two columns of `ls -l` that were there because they were easy rather than because
/// anybody read them: a `dir`/`file` word, which the name already says by ending in `/`, and a
/// `rwxrwxr-x` mode, which is nine characters answering a question a file browser is rarely asked.
/// A single mark says the same thing in one column and leaves the row to the name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Icons {
    pub directory: String,
    pub file: String,
    /// Per-extension marks, extension (without the dot, lowercased) to what is drawn.
    ///
    /// A `Vec` in declared order rather than a map, like [`super::Settings::abbr`]: it is installed once,
    /// and a map would only add a question about which of two entries for the same extension wins.
    pub by_extension: Vec<(String, String)>,
}

impl Default for Icons {
    fn default() -> Self {
        Icons {
            // Geometry rather than a glyph from a patched font: these have to land on a terminal
            // that has never heard of Nerd Fonts, which is most of the terminals a shell runs in.
            directory: "■".to_string(),
            file: "≡".to_string(),
            // Empty on purpose. Per-extension marks are configuration, not a list maintained here.
            by_extension: Vec::new(),
        }
    }
}

impl Icons {
    /// The mark for a name, by extension when one is configured and by kind otherwise.
    pub fn of(&self, name: &str, directory: bool) -> &str {
        if directory {
            return &self.directory;
        }
        // `Path::extension` rather than splitting on the last dot: `.gitignore` is a name that
        // begins with one, not a file with a `gitignore` extension, and it should read as a file.
        if let Some(extension) = std::path::Path::new(name)
            .extension()
            .and_then(|e| e.to_str())
        {
            let wanted = extension.to_ascii_lowercase();
            if let Some((_, icon)) = self.by_extension.iter().find(|(ext, _)| *ext == wanted) {
                return icon;
            }
        }
        &self.file
    }
}

impl Default for Nav {
    fn default() -> Self {
        Nav {
            fullscreen: true,
            position: crate::ask::chrome::Place::Center,
            width: 0,
            border: crate::ask::Border::None,
            border_fg: None,
            border_fit: crate::ask::chrome::Fit::Content,
            legend: false,
            legend_gap: 1,
            padding_x: 1,
            padding_y: 0,
            height: 0,
            hidden: false,
            filter_at: crate::ask::Where::Bottom,
            // **Downward, because the path is above it.** A reversed list grows towards the filter
            // and leaves its unused rows at the top — which is right for the history finder, whose
            // filter is the only thing above it, and wrong here: those rows land between the path
            // and the first entry. On a tall terminal that was seven blank lines splitting the
            // widget in half.
            reverse: false,
            scanner: true,
            icons: Icons::default(),
            type_nav: TypeNav::default(),
        }
    }
}
