//! What a sync says it did.
//!
//! Three storages produce three differently-shaped reports; a person wants one shape. So each is
//! flattened into [`Moved`] here, and printed in one format — which is also what lets the far end's
//! line be echoed under ours without looking like a different program wrote it.

use super::Part;

/// What changed, on each machine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Moved {
    pub added_here: usize,
    pub updated_here: usize,
    pub deleted_here: usize,
    pub added_there: usize,
    pub updated_there: usize,
    pub deleted_there: usize,
    pub unchanged: usize,
}

impl Moved {
    pub fn from_history(report: &oslo::track::SyncReport) -> Moved {
        Moved {
            added_here: report.added_left,
            updated_here: report.updated_left,
            deleted_here: report.deleted_left,
            added_there: report.added_right,
            updated_there: report.updated_right,
            deleted_there: report.deleted_right,
            unchanged: report.unchanged,
        }
    }

    pub fn from_macros(report: &oslo::macros::sync::MacroReport) -> Moved {
        Moved {
            added_here: report.added_left,
            updated_here: report.updated_left,
            deleted_here: report.deleted_left,
            added_there: report.added_right,
            updated_there: report.updated_right,
            deleted_there: report.deleted_right,
            unchanged: report.unchanged,
        }
    }

    #[cfg(feature = "secrets")]
    pub fn from_secrets(report: &oslo::secrets::sync::SecretReport) -> Moved {
        Moved {
            added_here: report.added_left,
            updated_here: report.updated_left,
            deleted_here: report.deleted_left,
            added_there: report.added_right,
            updated_there: report.updated_right,
            deleted_there: report.deleted_right,
            unchanged: report.unchanged,
        }
    }

    fn quiet(&self) -> bool {
        self.added_here == 0
            && self.updated_here == 0
            && self.deleted_here == 0
            && self.added_there == 0
            && self.updated_there == 0
            && self.deleted_there == 0
    }
}

/// One line per part, and a second only when something actually moved.
///
/// **A quiet part says so in one line.** Three parts each printing three lines of zeroes is a wall
/// of nothing, and the second sync of the day is always that.
pub fn say(part: Part, moved: &Moved) {
    if moved.quiet() {
        println!("{:<9} unchanged {}", part.word(), moved.unchanged);
        return;
    }
    println!(
        "{:<9} here +{} ~{} -{}   there +{} ~{} -{}   unchanged {}",
        part.word(),
        moved.added_here,
        moved.updated_here,
        moved.deleted_here,
        moved.added_there,
        moved.updated_there,
        moved.deleted_there,
        moved.unchanged,
    );
}

/// The far end says nothing.
///
/// **Because this side already said it.** The merge here computes both directions — that is what
/// `there +2 ~0 -0` above is — so the oslo over there repeating its half is the same line twice.
/// Its standard output is captured by ssh in any case, and the one thing it could add, a change
/// made over there between our two round trips, is carried by the merge and shown by the next sync.
pub fn say_far(_part: Part, _moved: &Moved) {}
