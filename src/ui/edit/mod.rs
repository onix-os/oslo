//! Oslo's native line editor.
//!
//! * [`buffer`] — the text and the cursor. Pure; every edit is a method and a unit test.
//! * [`layout`] — text, prompt and terminal width to the frame that draws them. Pure.
//! * [`session`] — key handling and terminal redraws.

pub mod buffer;
pub mod display;
pub mod keymap;
pub mod layout;
pub mod screen;
pub mod session;
pub mod vi;
