//! oslo's own line editor.
//!
//! Built to replace rustyline, and the reason is not the ~170KB. rustyline owns the *layout* of
//! the row it edits: it measures the prompt once, never redraws it, and cannot be asked to. Every
//! consequence of that is a workaround somewhere in this crate — the pre-rendered prompt variants
//! in [`crate::interactive::row`] that must all be the same printed width, the right prompt drawn
//! from inside the highlighter because that is the only place a cursor move does not confuse it,
//! the OSC markers dropped from `$PS1` because they are counted as visible width, and the finder
//! handing its choice back by faking an interrupt.
//!
//! Owning the layout removes the category, not the instances.
//!
//! # Shape
//!
//! * [`buffer`] — the text and the cursor. Pure; every edit is a method and a unit test.
//! * [`layout`] — text, prompt and terminal width to the frame that draws them. Pure.
//!
//! The terminal half is already oslo's: raw mode and key decoding in [`crate::interactive::term`],
//! and the scroll-safe redraw discipline in [`crate::interactive::paint`].

pub mod buffer;
pub mod keymap;
pub mod layout;
pub mod screen;
pub mod session;
