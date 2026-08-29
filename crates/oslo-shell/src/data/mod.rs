//! The structured half of a pipeline.
//!
//! A command in oslo can produce two things: text for a person, and rows for the next command.
//! This module is the second one — the values, and the two ways of writing them down.
//!
//! Nothing here knows about pipelines yet; that is the next stage. It is useful on its own because
//! the shell's own tools already build row tables by hand, and this gives them one shape with one
//! ordering and one renderer instead of several.
//!
//! The design, and the reasoning behind every choice here, is in
//! `docs/features/structured-pipelines.md`.

/// What a stage's columns will be, worked out before it runs.
pub mod columns;
/// Tools a config registered, consulted by the pipeline.
pub mod custom;
/// A cell as Lua sees it — the one converter both the filter and a Lua tool read it through.
pub mod lua;
/// Reaching into a row — `metadata.name`, `images.0` — for every verb that takes a column.
pub mod path;
pub mod plan;
/// System facts as rows, expressed as Lua values so both languages read the same table.
pub mod rows;
pub mod tool;
pub mod tools;
pub mod value;

pub use plan::{Shape, Sink, Stage, entered_structured_path, plan, reset_structured_path};
pub use value::{Record, Val, human_duration, human_size, render_display, render_transport};
