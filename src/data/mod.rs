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
//! `docs/research/dual-channel-pipe.md`.

pub mod plan;
pub mod tool;
pub mod tools;
pub mod value;

pub use plan::{Shape, Sink, Stage, entered_structured_path, plan, reset_structured_path};
pub use value::{Record, Val, human_duration, human_size, render_display, render_transport};
