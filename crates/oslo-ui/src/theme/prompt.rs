//! `oslo.theme.prompt` — the styles the built-in prompt draws with.

use super::{Color, Style};

/// Colours the built-in prompt draws with, when no Lua prompt has replaced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub cwd: Style,
    pub host: Style,
    pub user: Style,
    pub git: Style,
    pub ok: Style,
    pub failed: Style,
    /// The clock and the duration in the right prompt. Dim on purpose: they are there to be
    /// glanced at, not read, and a right prompt that competes with the command is a nuisance.
    pub aside: Style,
    /// The `>>>` in front of a continuation line.
    ///
    /// Its own style because a continuation line is a *different kind of row* from a prompt: it is
    /// the middle of something you are still writing, and colouring it like the prompt makes a
    /// half-finished block look like four separate commands that already ran.
    pub continuation: Style,
}

impl Default for Prompt {
    fn default() -> Self {
        let basic = |index: u8| {
            Style::fg(Color::Basic {
                index,
                bright: false,
            })
        };
        Prompt {
            cwd: Style {
                bold: true,
                ..basic(4)
            },
            host: Style {
                bold: true,
                ..basic(5)
            },
            user: basic(6),
            git: basic(2),
            ok: Style {
                bold: true,
                ..basic(2)
            },
            // Dim, and the same colour as the language segment: it says "still the same block"
            // rather than competing with the text being typed.
            continuation: Style {
                dim: true,
                ..basic(2)
            },
            failed: Style {
                bold: true,
                ..basic(1)
            },
            aside: Style {
                dim: true,
                ..basic(7)
            },
        }
    }
}
