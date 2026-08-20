//! What a stream coordinate will become, shown before Enter.
//!
//! ```text
//! wc -l {-1:0:1}
//!                ↳ one.txt
//! ```
//!
//! # Only the session axis can be shown, and that is not a limitation to hide
//!
//! `{-n:…}` reaches back through the *session* — previous command lines, which are recorded and
//! cost nothing to read. `{n:…}` reaches back through *this pipeline*, and at the moment you are
//! typing, the stage it names has not run. Resolving it would mean executing the upstream on every
//! keystroke, which is not a preview, it is running your pipeline to find out what it does.
//!
//! So a pipeline coordinate is answered with `at run time` rather than a value. That is deliberate:
//! silence would read as "not recognised", which is the one thing the preview exists to disprove.
//!
//! # Drawn, never accepted
//!
//! This is a third fallback under the suggestion and the repair in [`super::frame::draw`], and it
//! is reached only for *drawing*. `Bound::AcceptHint` asks `take_hint` and `take_repair` for their
//! own text and never for what was drawn, so there is no key that can put `↳ one.txt` into the
//! buffer. A preview is a fact about the line, not a proposal to change it.

use crate::highlight::lex::{Role, lex};
use oslo_base::coords::{self, Coord, Sel, Subject};

/// The annotation for the last coordinate in `line`, if it has one.
pub(super) fn preview(line: &str) -> Option<String> {
    // **The lexer's own answer**, so the thing previewed is exactly the thing painted. A second
    // scan here would be a second opinion about what a coordinate is, and the two would drift.
    let span = lex(line)
        .into_iter()
        .rev()
        .find(|s| s.role == Role::Coordinate)?;
    let inside = span.text.strip_prefix('{')?.strip_suffix('}')?;
    let coord = coords::parse(inside)?;

    let values = match reach(&coord) {
        // Forward through this pipeline: the stage has not run, and running it to find out is not
        // something a keystroke may do.
        Reach::Pipeline => return Some("at run time".to_string()),
        Reach::Session(back) => resolve(&coord, back)?,
    };
    Some(shown(&values))
}

/// Which way a coordinate reaches, and how far.
enum Reach {
    Pipeline,
    Session(usize),
}

/// Zero and up walk back through this pipeline; below zero walks back through the session.
fn reach(coord: &Coord) -> Reach {
    let at = match coord.stream {
        Sel::At(at) => at,
        // A range of streams takes the first, as the substitution does.
        Sel::Span { from, .. } => from.unwrap_or(0),
    };
    match at < 0 {
        true => Reach::Session(at.unsigned_abs()),
        false => Reach::Pipeline,
    }
}

/// What the coordinate reads out of the remembered prompt line.
///
/// **Reaching past the ring is empty, not silent.** The substitution answers nothing there and runs
/// the command anyway, so a preview that drew no annotation would hide exactly the case worth
/// catching — an argument that is about to go missing.
fn resolve(coord: &Coord, back: usize) -> Option<Vec<String>> {
    let Some(line) = oslo_base::prompts::back(back) else {
        return Some(Vec::new());
    };
    Some(match coord.subject {
        // A command line is one line, and `{%…}` addresses its words.
        Subject::Command => {
            let words: Vec<String> = line.split_whitespace().map(str::to_string).collect();
            coords::select_words(coord, &words)
        }
        Subject::Output => coords::select(coord, &line),
    })
}

/// The values as one short annotation.
///
/// **Bounded, and honest about what it cut.** A coordinate can name every word of a line, and a
/// preview that pushed the prompt onto a second row to show forty of them would cost more than it
/// tells you. Three, then a count of the rest.
fn shown(values: &[String]) -> String {
    const SHOWN: usize = 3;
    const WIDTH: usize = 48;

    if values.is_empty() {
        return "nothing there".to_string();
    }
    let mut text = values
        .iter()
        .take(SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    if text.chars().count() > WIDTH {
        text = text.chars().take(WIDTH - 1).collect::<String>() + "…";
    }
    match values.len() > SHOWN {
        true => format!("{text} … ({} values)", values.len()),
        false => text,
    }
}

#[cfg(test)]
#[path = "preview/tests.rs"]
mod tests;
