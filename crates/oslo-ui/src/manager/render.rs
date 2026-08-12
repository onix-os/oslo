//! Drawing the manager: one full-screen frame per keystroke.
//!
//! Almost none of it is drawn here. The rows, the striping, the columns, the match marks, the
//! three-row surface and the counter all come from [`Preset::History`] — the same look the history
//! finder is drawn with, which is what makes the two screens the same screen. This module decides
//! what the columns *say*:
//!
//! ```text
//!   3d   alias   gs      git status --short              #git #system
//!   └─ when      └─ kind, so a list of four kinds can be read down the column
//!                        └─ the name, then its first line
//!                                                        └─ its tags, hard right
//! ```
//!
//! A macro that is off is drawn muted and marked, because a list where a disabled row looks exactly
//! like a live one is a list that answers the wrong question.

use super::{Item, Source};
use crate::ask::look::{Look, Preset, Row, View};
use crate::paint::{SYNC_BEGIN, SYNC_END};
use crate::theme;

/// Rows the input surface takes: a blank, the query, a blank. The finder's shape, deliberately.
const SURFACE_ROWS: usize = 3;
const CHROME_ROWS: usize = SURFACE_ROWS + 3;
const SCREEN_MARGIN: usize = 1;

pub struct Frame<'a> {
    pub rows: &'a [Item],
    pub selected: usize,
    pub offset: usize,
    pub query: &'a str,
    pub elapsed_ms: u64,
    pub confirm: Option<bool>,
    pub source: Source,
    pub tag: Option<String>,
    pub total: usize,
    pub cols: usize,
    pub rows_available: usize,
    pub now: i64,
}

/// How many rows remain for the list after the input and its margins.
pub(super) fn visible_rows(rows: usize) -> usize {
    rows.saturating_sub(CHROME_ROWS).max(1)
}

/// Unix seconds now, for the age column.
pub(super) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0)
}

/// How long ago, in the two or three characters the column has.
///
/// The finder's `ago` measures a moment in the past few hours; this measures how long you have kept
/// something, which is usually months. Same column, same shape, one more unit on the end.
pub fn ago(now: i64, then: i64) -> String {
    if then <= 0 {
        return "—".to_string();
    }
    let seconds = (now - then).max(0);
    match seconds {
        s if s < 60 => "now".to_string(),
        s if s < 3_600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3_600),
        s if s < 2_592_000 => format!("{}d", s / 86_400),
        s if s < 31_536_000 => format!("{}mo", s / 2_592_000),
        s => format!("{}y", s / 31_536_000),
    }
}

/// The look, with only the two things the preset cannot know set here.
fn look_of(f: &Frame<'_>) -> Look {
    let mut look = Preset::History.look();
    look.right = format!("{} @ {{badge}} || {{n}}/{{total}} ", f.source.label());
    look.badge = match &f.tag {
        Some(tag) => format!("[#{tag}]"),
        None => "[all]".to_string(),
    };
    // **An empty list says which empty list it is.** The screen opens on an empty database on
    // purpose — the database is not the only source, and Tab is how you reach the other one — so the
    // one place a person is already looking says where the rest of it went.
    if f.rows.is_empty() && f.query.is_empty() {
        look.placeholder = match (f.source, f.tag.is_some()) {
            (_, true) => "nothing with this tag — ← → for another".to_string(),
            (Source::Stored, false) => {
                "nothing stored — tab for what your config defines".to_string()
            }
            (Source::Elsewhere, false) => {
                "your config defines none — tab back to what is stored".to_string()
            }
        };
    }
    look
}

pub fn frame(f: &Frame<'_>) -> String {
    let look = look_of(f);
    let visible = visible_rows(f.rows_available);

    let rows: Vec<Row> = f
        .rows
        .iter()
        .map(|item| row_of(item, &look, f.now))
        .collect();
    let view = View {
        selected: f.selected,
        offset: f.offset,
        height: visible,
        query: f.query,
        matched: f.rows.len(),
        total: f.total,
        marked: 0,
        cols: f.cols,
        filtering: f.confirm.is_none(),
        elapsed_ms: f.elapsed_ms,
    };

    let mut body = look.rows(&rows, &view);
    if let Some(yes) = f.confirm {
        let pager = &theme::current().pager;
        body.extend(std::iter::repeat_n(String::new(), look.gap));
        let question = f
            .rows
            .get(f.selected)
            .map(|item| format!("forget the {} {}?", item.kind, item.name))
            .unwrap_or_else(|| "forget it?".to_string());
        let depth = theme::depth();
        body.extend((0..SURFACE_ROWS).map(|row| {
            crate::finder::render::confirm_row(row, yes, &question, pager, f.cols, depth)
        }));
    }

    let mut out = String::from(SYNC_BEGIN);
    out.push_str("\x1b[H");
    for _ in 0..SCREEN_MARGIN {
        out.push_str("\x1b[2K\r\n");
    }
    for row in &body {
        out.push_str("\x1b[2K");
        out.push_str(row);
        out.push_str("\r\n");
    }
    out.push_str("\x1b[2K");
    out.push_str(SYNC_END);
    out
}

/// One macro as a row of the list.
fn row_of(item: &Item, look: &Look, now: i64) -> Row {
    let tags: String = item
        .tags
        .iter()
        .map(|tag| format!("#{tag}"))
        .collect::<Vec<_>>()
        .join(" ");
    // The state marker sits where the checkbox does in a multi-select list, because it is the same
    // question — is this one in or out — asked of a different thing.
    let lead = match (item.active, item.session_off) {
        (false, _) => "✗".to_string(),
        (true, true) => "·".to_string(),
        (true, false) => " ".to_string(),
    };
    Row {
        meta: vec![ago(now, item.created), item.kind.clone()],
        lead,
        trail: tags,
        // Muted while it is off: a list where a disabled row looks exactly like a live one answers
        // the wrong question.
        tint: (!item.on()).then_some(look.muted),
        ..Row::new(format!("{}  {}", item.name, item.first))
    }
}

// The confirmation box is `finder::render::confirm_row`, question and all. Two screens drawing the
// same box from two copies of the same arithmetic would be two boxes that eventually differ.
