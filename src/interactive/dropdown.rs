use nix::sys::termios::{LocalFlags, SetArg, tcgetattr, tcsetattr};
use std::io::{self, Read, Write};

#[derive(Debug, Clone)]
pub struct CompletionCandidate {
    pub display: String,
    pub replacement: String,
    pub description: Option<String>,
    pub kind: Option<String>,
}

impl CompletionCandidate {
    pub fn new(display: String, replacement: String, description: Option<String>) -> Self {
        Self {
            display,
            replacement,
            description,
            kind: None,
        }
    }

    pub fn icon(&self) -> &'static str {
        if let Some(ref k) = self.kind {
            match k.as_str() {
                "subcommand" => "🏷️ ",
                "flag" => "🚩 ",
                "dir" => "📁 ",
                "file" => "📄 ",
                "builtin" => "⚡ ",
                "variable" => "💲 ",
                _ => "⚙️ ",
            }
        } else if self.display.starts_with('-') {
            "🚩 "
        } else if self.display.ends_with('/') {
            "📁 "
        } else if self.display.starts_with('$') {
            "💲 "
        } else {
            "⚙️ "
        }
    }
}

pub fn visible_len(s: &str) -> usize {
    let mut len = 0;
    let mut in_esc = false;
    for c in s.chars() {
        if c == '\x1b' {
            in_esc = true;
        } else if in_esc {
            if c == 'm' {
                in_esc = false;
            }
        } else {
            len += 1;
        }
    }
    len
}

pub fn render_vertical_dropdown(
    candidates: &[CompletionCandidate],
    selected_idx: usize,
    max_visible: usize,
    indent_cols: usize,
) -> (String, usize) {
    if candidates.is_empty() {
        return (String::new(), 0);
    }

    let max_visible = max_visible.min(8);
    let start = (selected_idx / max_visible) * max_visible;
    let end = (start + max_visible).min(candidates.len());
    let visible_slice = &candidates[start..end];

    let max_label_len = visible_slice
        .iter()
        .map(|c| c.display.len() + 4)
        .max()
        .unwrap_or(15)
        .max(16);

    let max_desc_len = visible_slice
        .iter()
        .map(|c| c.description.as_deref().unwrap_or("").len())
        .max()
        .unwrap_or(0);

    let has_desc = max_desc_len > 0;
    let desc_col_width = if has_desc { max_desc_len.max(25) } else { 0 };

    let total_box_width = if has_desc {
        max_label_len + desc_col_width + 7
    } else {
        max_label_len + 5
    };

    let mut out = String::new();
    let num_lines = visible_slice.len() + 2; // includes top and bottom box borders
    let indent = " ".repeat(indent_cols);

    // Top border of IRIS popover box
    out.push_str("\r\n");
    let header_title = format!(" Suggestions ({}/{}) ", selected_idx + 1, candidates.len());
    let remaining_border = total_box_width.saturating_sub(header_title.len() + 2);
    out.push_str(&format!(
        "{}\x1b[38;5;240m╭─\x1b[1;36m{}\x1b[0m\x1b[38;5;240m{}╮\x1b[0m\x1b[K",
        indent,
        header_title,
        "─".repeat(remaining_border)
    ));

    // Render items
    for (i, cand) in visible_slice.iter().enumerate() {
        let abs_idx = start + i;
        let icon = cand.icon();
        let label = format!("{}{}", icon, cand.display);
        let desc = cand.description.as_deref().unwrap_or("");

        out.push_str("\r\n");

        if abs_idx == selected_idx {
            // Selected item: IRIS Indigo/Purple background highlight (\x1b[48;5;62m)
            if has_desc {
                out.push_str(&format!(
                    "{}\x1b[38;5;240m│\x1b[0m \x1b[48;5;62m\x1b[1;97m ▶ {:<label_w$} \x1b[36m {:<desc_w$} \x1b[0m \x1b[38;5;240m│\x1b[0m\x1b[K",
                    indent,
                    label,
                    desc,
                    label_w = max_label_len,
                    desc_w = desc_col_width
                ));
            } else {
                out.push_str(&format!(
                    "{}\x1b[38;5;240m│\x1b[0m \x1b[48;5;62m\x1b[1;97m ▶ {:<label_w$} \x1b[0m \x1b[38;5;240m│\x1b[0m\x1b[K",
                    indent,
                    label,
                    label_w = max_label_len
                ));
            }
        } else {
            // Unselected item: Dark slate background (\x1b[48;5;236m)
            if has_desc {
                out.push_str(&format!(
                    "{}\x1b[38;5;240m│\x1b[0m \x1b[48;5;236m\x1b[37m   {:<label_w$} \x1b[38;5;245m {:<desc_w$} \x1b[0m \x1b[38;5;240m│\x1b[0m\x1b[K",
                    indent,
                    label,
                    desc,
                    label_w = max_label_len,
                    desc_w = desc_col_width
                ));
            } else {
                out.push_str(&format!(
                    "{}\x1b[38;5;240m│\x1b[0m \x1b[48;5;236m\x1b[37m   {:<label_w$} \x1b[0m \x1b[38;5;240m│\x1b[0m\x1b[K",
                    indent,
                    label,
                    label_w = max_label_len
                ));
            }
        }
    }

    // Bottom border of IRIS popover box
    out.push_str("\r\n");
    let footer_text = " Tab/Enter to select ";
    let remaining_footer = total_box_width.saturating_sub(footer_text.len() + 2);
    out.push_str(&format!(
        "{}\x1b[38;5;240m╰─\x1b[90m{}\x1b[0m\x1b[38;5;240m{}╯\x1b[0m\x1b[K",
        indent,
        footer_text,
        "─".repeat(remaining_footer)
    ));

    (out, num_lines)
}

pub struct DropdownMenu {
    pub candidates: Vec<CompletionCandidate>,
    pub selected_index: usize,
    pub max_visible: usize,
    pub indent_cols: usize,
}

impl DropdownMenu {
    pub fn new(candidates: Vec<CompletionCandidate>, indent_cols: usize) -> Self {
        Self {
            candidates,
            selected_index: 0,
            max_visible: 8,
            indent_cols,
        }
    }

    pub fn select_interactive(
        candidates: Vec<CompletionCandidate>,
        indent_cols: usize,
    ) -> Option<CompletionCandidate> {
        if candidates.is_empty() {
            return None;
        }
        if candidates.len() == 1 {
            return Some(candidates[0].clone());
        }

        let mut menu = Self::new(candidates, indent_cols);

        let stdin = io::stdin();
        let orig_termios = tcgetattr(&stdin).ok()?;
        let mut raw_termios = orig_termios.clone();
        raw_termios.local_flags.remove(LocalFlags::ICANON);
        raw_termios.local_flags.remove(LocalFlags::ECHO);
        let _ = tcsetattr(&stdin, SetArg::TCSANOW, &raw_termios);

        let mut stdout = io::stdout();

        let selected = loop {
            let (rendered, num_lines) = render_vertical_dropdown(
                &menu.candidates,
                menu.selected_index,
                menu.max_visible,
                menu.indent_cols,
            );
            let _ = write!(stdout, "{}", rendered);
            let _ = stdout.flush();

            let mut buf = [0u8; 3];
            let n = io::stdin().read(&mut buf).unwrap_or(0);

            // Move back up over rendered dropdown lines (num_lines) and erase completely below prompt
            let _ = write!(stdout, "\x1b[{}A\r\x1b[J", num_lines);
            let _ = stdout.flush();

            if n == 0 {
                break None;
            }

            if n == 1 {
                match buf[0] {
                    13 | 10 | 32 => {
                        // Enter or Space
                        break Some(menu.candidates[menu.selected_index].clone());
                    }
                    9 => {
                        // Tab cycles down
                        menu.selected_index = (menu.selected_index + 1) % menu.candidates.len();
                    }
                    27 => {
                        // Esc
                        break None;
                    }
                    _ => break None,
                }
            } else if n == 3 && buf[0] == 27 && buf[1] == 91 {
                match buf[2] {
                    65 => {
                        // Up Arrow
                        if menu.selected_index > 0 {
                            menu.selected_index -= 1;
                        } else {
                            menu.selected_index = menu.candidates.len() - 1;
                        }
                    }
                    66 => {
                        // Down Arrow
                        menu.selected_index = (menu.selected_index + 1) % menu.candidates.len();
                    }
                    _ => break None,
                }
            } else {
                break None;
            }
        };

        let _ = tcsetattr(&stdin, SetArg::TCSANOW, &orig_termios);
        selected
    }
}
