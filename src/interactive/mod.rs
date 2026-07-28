pub mod dropdown;
pub mod prompt;
pub mod spec;

use crate::env::Environment;
use dropdown::{CompletionCandidate, DropdownMenu};
use rustyline::completion::{Completer, FilenameCompleter, Pair};
use rustyline::highlight::Highlighter;
use rustyline::hint::{Hinter, HistoryHinter};
use rustyline::validate::{ValidationContext, ValidationResult, Validator};
use rustyline::{Context, Helper};
use spec::SpecRegistry;
use std::borrow::Cow;
use std::collections::HashSet;
use std::fs;
use std::sync::{Arc, Mutex};

pub struct RushHelper {
    env: Arc<Mutex<Environment>>,
    _filename_completer: FilenameCompleter,
    history_hinter: HistoryHinter,
    spec_registry: SpecRegistry,
}

impl RushHelper {
    pub fn new(env: Arc<Mutex<Environment>>) -> Self {
        Self {
            env,
            _filename_completer: FilenameCompleter::new(),
            history_hinter: HistoryHinter::new(),
            spec_registry: SpecRegistry::new(),
        }
    }

    fn collect_command_names(&self) -> HashSet<String> {
        let mut cmds = HashSet::new();
        let env_guard = self.env.lock().unwrap();

        // Builtins, straight from the registry rather than a copy that drifts out of date.
        for b in env_guard.builtin_names() {
            cmds.insert(b.to_string());
        }

        // Aliases
        for a in env_guard.get_aliases().keys() {
            cmds.insert(a.clone());
        }

        // Functions
        for f in env_guard.get_functions().keys() {
            cmds.insert(f.clone());
        }

        // Executables in PATH
        if let Some(path_var) = env_guard.get_var("PATH") {
            for dir in path_var.split(':') {
                if let Ok(entries) = fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        if let Ok(file_type) = entry.file_type()
                            && (file_type.is_file() || file_type.is_symlink())
                        {
                            cmds.insert(entry.file_name().to_string_lossy().to_string());
                        }
                    }
                }
            }
        }

        cmds
    }
}

impl Completer for RushHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let (start, word) = extract_current_word(line, pos);
        let mut cand_objs = Vec::new();

        // 1. If completing the first word (command name)
        if start == 0 || line[..start].trim().is_empty() {
            let candidates = self.collect_command_names();
            for cmd in candidates {
                if cmd.starts_with(word) {
                    let desc = self
                        .spec_registry
                        .find_spec(&cmd)
                        .map(|s| s.description.to_string());
                    let kind = if self.env.lock().unwrap().is_builtin(&cmd) {
                        Some("builtin".to_string())
                    } else {
                        Some("command".to_string())
                    };
                    cand_objs.push(CompletionCandidate {
                        display: cmd.clone(),
                        replacement: cmd,
                        description: desc,
                        kind,
                    });
                }
            }
        } else if let Some(var_prefix) = word.strip_prefix('$') {
            // 2. Environment variables starting with $
            let env_guard = self.env.lock().unwrap();
            for (k, _) in env_guard.get_all_vars() {
                if k.starts_with(var_prefix) {
                    let replacement = format!("${}", k);
                    cand_objs.push(CompletionCandidate {
                        display: replacement.clone(),
                        replacement,
                        description: Some("Environment variable".to_string()),
                        kind: Some("variable".to_string()),
                    });
                }
            }
        } else {
            // 3. Subcommand & Option completions from IRIS spec_registry (e.g. `git commit -`, `cargo build --`)
            let tokens: Vec<&str> = line[..start].split_whitespace().collect();
            if !tokens.is_empty() {
                let primary_cmd = tokens[0];
                let sub_tokens = &tokens[1..];
                let spec_matches =
                    self.spec_registry
                        .get_subcommand_suggestions(primary_cmd, sub_tokens, word);

                for (name, desc) in spec_matches {
                    let kind = if name.starts_with('-') {
                        Some("flag".to_string())
                    } else {
                        Some("subcommand".to_string())
                    };
                    cand_objs.push(CompletionCandidate {
                        display: name.to_string(),
                        replacement: name.to_string(),
                        description: Some(desc.to_string()),
                        kind,
                    });
                }
            }

            // 4. Fallback/Augment Path & Directory completion
            if cand_objs.is_empty() {
                let cmd_name = line[..start].trim();
                let is_cd_cmd = matches!(cmd_name, "cd" | "pushd");

                let (dir_path, prefix) = if let Some(slash_idx) = word.rfind('/') {
                    (&word[..=slash_idx], &word[slash_idx + 1..])
                } else {
                    ("", word)
                };

                let expand_dir = if let Some(rest) = dir_path.strip_prefix('~') {
                    match std::env::var("HOME") {
                        Ok(home) => format!("{}{}", home, rest),
                        Err(_) => dir_path.to_string(),
                    }
                } else if dir_path.is_empty() {
                    ".".to_string()
                } else {
                    dir_path.to_string()
                };

                if let Ok(entries) = fs::read_dir(&expand_dir) {
                    for entry in entries.flatten() {
                        let file_name = entry.file_name().to_string_lossy().to_string();
                        if file_name.starts_with(prefix) {
                            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                            if is_cd_cmd && !is_dir {
                                continue;
                            }

                            let display = if is_dir {
                                format!("{}/", file_name)
                            } else {
                                file_name.clone()
                            };

                            let replacement = format!("{}{}", dir_path, display);
                            let description = if is_dir {
                                Some("Directory".to_string())
                            } else {
                                Some("File".to_string())
                            };
                            let kind = if is_dir {
                                Some("dir".to_string())
                            } else {
                                Some("file".to_string())
                            };

                            cand_objs.push(CompletionCandidate {
                                display,
                                replacement,
                                description,
                                kind,
                            });
                        }
                    }
                }
            }
        }

        cand_objs.sort_by(|a, b| {
            let score_a = self.spec_registry.frecency.get_score(&a.display);
            let score_b = self.spec_registry.frecency.get_score(&b.display);
            score_b
                .partial_cmp(&score_a)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.display.cmp(&b.display))
        });

        if cand_objs.len() > 1 {
            let prompt_str = prompt::render_default_left_prompt(0);
            let prompt_len = dropdown::visible_len(&prompt_str);
            let line_prefix_len = dropdown::visible_len(&line[..start]);
            let indent_cols = prompt_len + line_prefix_len;

            if let Some(selected) = DropdownMenu::select_interactive(cand_objs, indent_cols) {
                return Ok((
                    start,
                    vec![Pair {
                        display: selected.display,
                        replacement: selected.replacement,
                    }],
                ));
            }
            return Ok((start, Vec::new()));
        } else if cand_objs.len() == 1 {
            let item = cand_objs.remove(0);
            return Ok((
                start,
                vec![Pair {
                    display: item.display,
                    replacement: item.replacement,
                }],
            ));
        }

        Ok((start, Vec::new()))
    }
}

impl Hinter for RushHelper {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, ctx: &Context<'_>) -> Option<Self::Hint> {
        if line.is_empty() || pos < line.len() {
            return None;
        }

        // Try history hinter first
        if let Some(h) = self.history_hinter.hint(line, pos, ctx) {
            return Some(h);
        }

        // Try auto-suggesting matching command name if typing first word
        let (start, word) = extract_current_word(line, pos);
        if start == 0 && !word.is_empty() {
            let candidates = self.collect_command_names();
            let mut matches: Vec<&String> = candidates
                .iter()
                .filter(|c| c.starts_with(word) && *c != word)
                .collect();
            matches.sort();
            if let Some(first_match) = matches.first() {
                return Some(first_match[word.len()..].to_string());
            }
        }

        None
    }
}

impl Highlighter for RushHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        if line.is_empty() {
            return Cow::Borrowed(line);
        }

        let mut highlighted = String::new();
        let tokens = tokenize_for_highlight(line);

        for (tok_str, tok_type) in tokens.iter() {
            match tok_type {
                TokenType::Command => {
                    let env_guard = self.env.lock().unwrap();
                    let is_valid = env_guard.is_builtin(tok_str)
                        || env_guard.get_alias(tok_str).is_some()
                        || env_guard.get_function(tok_str).is_some()
                        || which::which(tok_str).is_ok();

                    if is_valid {
                        highlighted.push_str(&format!("\x1b[1;32m{}\x1b[0m", tok_str)); // Bold Green
                    } else {
                        highlighted.push_str(&format!("\x1b[1;31m{}\x1b[0m", tok_str)); // Bold Red
                    }
                }
                TokenType::Flag => {
                    highlighted.push_str(&format!("\x1b[36m{}\x1b[0m", tok_str)); // Cyan
                }
                TokenType::String => {
                    highlighted.push_str(&format!("\x1b[33m{}\x1b[0m", tok_str)); // Yellow
                }
                TokenType::Variable => {
                    highlighted.push_str(&format!("\x1b[35m{}\x1b[0m", tok_str)); // Magenta
                }
                TokenType::Operator => {
                    highlighted.push_str(&format!("\x1b[1;37m{}\x1b[0m", tok_str)); // Bold White
                }
                TokenType::Plain => {
                    highlighted.push_str(tok_str);
                }
            }
        }

        Cow::Owned(highlighted)
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        // Render fish-style ghost suggestion text in dim gray
        Cow::Owned(format!("\x1b[90m{}\x1b[0m", hint))
    }
}

impl Validator for RushHelper {
    fn validate(&self, ctx: &mut ValidationContext<'_>) -> rustyline::Result<ValidationResult> {
        let input = ctx.input();
        let mut in_single_quote = false;
        let mut in_double_quote = false;

        for ch in input.chars() {
            match ch {
                '\'' if !in_double_quote => in_single_quote = !in_single_quote,
                '"' if !in_single_quote => in_double_quote = !in_double_quote,
                _ => {}
            }
        }

        if in_single_quote || in_double_quote {
            Ok(ValidationResult::Incomplete)
        } else {
            Ok(ValidationResult::Valid(None))
        }
    }
}

impl Helper for RushHelper {}

#[derive(Debug, PartialEq, Eq)]
enum TokenType {
    Command,
    Flag,
    String,
    Variable,
    Operator,
    Plain,
}

fn tokenize_for_highlight(line: &str) -> Vec<(String, TokenType)> {
    let mut result = Vec::new();
    let mut chars = line.chars().peekable();
    let mut is_first_word = true;

    while let Some(&ch) = chars.peek() {
        if ch.is_whitespace() {
            let mut space_str = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    space_str.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            result.push((space_str, TokenType::Plain));
            continue;
        }

        if matches!(ch, '|' | '&' | ';' | '<' | '>') {
            let mut op_str = String::new();
            op_str.push(ch);
            chars.next();
            if let Some(&next_ch) = chars.peek()
                && ((ch == '|' && next_ch == '|')
                    || (ch == '&' && next_ch == '&')
                    || (ch == '>' && next_ch == '>'))
            {
                op_str.push(next_ch);
                chars.next();
            }
            result.push((op_str, TokenType::Operator));
            is_first_word = true;
            continue;
        }

        if ch == '\'' || ch == '"' {
            let quote = ch;
            let mut str_lit = String::new();
            str_lit.push(quote);
            chars.next();
            while let Some(&c) = chars.peek() {
                str_lit.push(c);
                chars.next();
                if c == quote {
                    break;
                }
            }
            result.push((str_lit, TokenType::String));
            is_first_word = false;
            continue;
        }

        if ch == '$' {
            let mut var_str = String::new();
            var_str.push(ch);
            chars.next();
            while let Some(&c) = chars.peek() {
                if c.is_alphanumeric() || c == '_' || c == '?' || c == '!' || c == '#' {
                    var_str.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            result.push((var_str, TokenType::Variable));
            is_first_word = false;
            continue;
        }

        let mut word_str = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() || matches!(c, '|' | '&' | ';' | '<' | '>' | '\'' | '"' | '$') {
                break;
            }
            word_str.push(c);
            chars.next();
        }

        if is_first_word {
            result.push((word_str, TokenType::Command));
            is_first_word = false;
        } else if word_str.starts_with('-') {
            result.push((word_str, TokenType::Flag));
        } else {
            result.push((word_str, TokenType::Plain));
        }
    }

    result
}

fn extract_current_word(line: &str, pos: usize) -> (usize, &str) {
    let sub = &line[..pos];
    if let Some(idx) =
        sub.rfind(|c: char| c.is_whitespace() || matches!(c, '|' | '&' | ';' | '<' | '>'))
    {
        let start = idx + 1;
        (start, &sub[start..])
    } else {
        (0, sub)
    }
}
