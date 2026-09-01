//! The registry: every name that can carry structure.

use super::*;

/// Declare every structured tool. Called once, at startup.
///
/// This is the *whole* vocabulary that can carry structure. Every name here is one oslo invented,
/// which is what makes the POSIX guarantee mechanical: a script written before oslo existed cannot
/// name any of them, so no edge of it can ever be planned as rows.
pub fn register_all() {
    crate::data::tool::register("df", Shape::Nothing, Shape::Rows);
    crate::data::tool::register("ps", Shape::Nothing, Shape::Rows);
    crate::data::tool::register("ls", Shape::Nothing, Shape::Rows);
    // The fourth producer, and the first that reads what the shell wrote about itself rather than
    // what the machine says. See `past`, and the note there on why the name collision is chosen.
    crate::data::tool::register("history", Shape::Nothing, Shape::Rows);
    crate::data::tool::register("where", Shape::Rows, Shape::Rows);
    // Operands, rows or bytes in; always rows out. See `tools::text` for why the shape is fixed.
    // `Any` is also what makes `text upper hello` a command on its own — see `bridge_at_the_tail`.
    #[cfg(feature = "text")]
    crate::data::tool::register("text", Shape::Any, Shape::Rows);
    // The bridge into structure. These take *bytes* — which is what an external command produces —
    // and manufacture rows, so they work with every program already installed.
    crate::data::tool::register("lines", Shape::Bytes, Shape::Rows);
    crate::data::tool::register("parse", Shape::Bytes, Shape::Rows);
    crate::data::tool::register("from", Shape::Bytes, Shape::Rows);
    // Somebody else's aligned output, with no pattern to write and nothing for them to agree to.
    crate::data::tool::register("detect-columns", Shape::Bytes, Shape::Rows);
    // The verbs. `cols` rather than `select`, which the parser refuses as a bash keyword.
    // `map` answers a row per row; `each` answers none and ends the pipeline. Two names because
    // they are two things — a flag on one would make "does this produce rows" a runtime question,
    // and the planner has to know it before anything runs.
    for name in [
        "cols", "get", "sort-by", "first", "final", "length", "each", "map", "reverse",
    ] {
        crate::data::tool::register(name, Shape::Rows, Shape::Rows);
    }
    // The verbs that make a stream smaller. See `summarise` for why these and not `join`.
    for name in [
        "group-by",
        "count",
        "distinct",
        "stats",
        "describe",
        "histogram",
        "reduce",
    ] {
        crate::data::tool::register(name, Shape::Rows, Shape::Rows);
    }
    // Reshaping: which columns a stream has, and which rows. See `reshape` for the twelve taken and
    // the ten deliberately not, which is a decision about the name budget rather than about effort.
    for name in [
        "reject",
        "rename",
        "insert",
        "update",
        "upsert",
        "flatten",
        "headers",
        "skip",
        "every",
        "enumerate",
        "compact",
        "default",
    ] {
        crate::data::tool::register(name, Shape::Rows, Shape::Rows);
    }
    // The verbs that need a second stream, which they name as a Lua expression because the pipeline
    // is a line. `lookup` rather than `join`, which is POSIX. See `second`.
    for name in ["lookup", "append", "merge"] {
        crate::data::tool::register(name, Shape::Rows, Shape::Rows);
    }
    // The viewer. `Rows -> Rows` like `each`, and like `each` it answers none of them: the shapes
    // are what the *planner* needs, and "produces nothing" is a fact about the run. Declaring
    // `Nothing` instead would leave `ps | explore | to json` on the byte path, where `to` is not a
    // command — a worse answer than an empty one.
    crate::data::tool::register("explore", Shape::Rows, Shape::Rows);
    // The way out. Rows in, bytes out — so `... | to json | jq .` works, and the structured world
    // is not a place you cannot leave.
    crate::data::tool::register("to", Shape::Rows, Shape::Bytes);
}
