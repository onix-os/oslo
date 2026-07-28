use crate::ast::{ParamExpansion, Word, WordPart};
use crate::env::Environment;
use crate::error::{Result, ShellError};
use crate::expand::arithmetic::eval_arithmetic;
use glob::glob;

/// Expand a word to a single string: parameters, command substitution, arithmetic and tilde,
/// but *not* field splitting or pathname expansion.
///
/// This is what `case` needs for both the subject and its patterns. POSIX excludes both of the
/// latter steps there, and applying them is actively wrong: globbing a pattern turns
/// `case foo in f*)` into a match against whatever files happen to be in the working directory,
/// so the branch silently stops firing depending on where you run the script.
///
/// Returns `(text, was_quoted)`; the flag lets callers that *do* want splitting reuse this.
fn expand_word_parts(env: &mut Environment, word: &Word) -> Result<(String, bool)> {
    let mut expanded_str = String::new();
    let mut is_quoted = false;

    for part in &word.parts {
        let (part_str, quoted) = expand_word_part(env, part)?;
        expanded_str.push_str(&part_str);
        if quoted {
            is_quoted = true;
        }
    }

    Ok((expanded_str, is_quoted))
}

/// Expand to exactly one string, skipping field splitting and globbing.
pub fn expand_word_to_string(env: &mut Environment, word: &Word) -> Result<String> {
    Ok(expand_word_parts(env, word)?.0)
}

pub fn expand_word(env: &mut Environment, word: &Word) -> Result<Vec<String>> {
    let (expanded_str, is_quoted) = expand_word_parts(env, word)?;

    if is_quoted {
        return Ok(vec![expanded_str]);
    }

    // Field splitting via IFS
    let ifs = env.get_var("IFS").unwrap_or(" \t\n").to_string();
    let fields: Vec<String> = if ifs.is_empty() {
        vec![expanded_str]
    } else {
        expanded_str
            .split(|c| ifs.contains(c))
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    };

    if fields.is_empty() {
        return Ok(Vec::new());
    }

    // Pathname expansion (globbing)
    let mut result = Vec::new();
    for field in fields {
        if (field.contains('*') || field.contains('?') || field.contains('['))
            && let Ok(paths) = glob(&field)
        {
            let mut matched = Vec::new();
            for entry in paths.flatten() {
                matched.push(entry.to_string_lossy().to_string());
            }
            if !matched.is_empty() {
                result.extend(matched);
                continue;
            }
        }
        result.push(field);
    }

    Ok(result)
}

fn expand_word_part(env: &mut Environment, part: &WordPart) -> Result<(String, bool)> {
    match part {
        WordPart::Literal(s) => Ok((s.clone(), false)),
        WordPart::SingleQuoted(s) => Ok((s.clone(), true)),
        WordPart::DoubleQuoted(parts) => {
            let mut res = String::new();
            for p in parts {
                let (sub, _) = expand_word_part(env, p)?;
                res.push_str(&sub);
            }
            Ok((res, true))
        }
        WordPart::Tilde(user) => {
            if user.is_empty() {
                let home = env.get_var("HOME").unwrap_or("/").to_string();
                Ok((home, false))
            } else {
                Ok((format!("~{}", user), false))
            }
        }
        WordPart::Variable {
            name,
            expansion_type,
        } => {
            let val = env.get_param(name);
            let result_str = match expansion_type {
                ParamExpansion::Normal => val.unwrap_or_default(),
                ParamExpansion::Length => val
                    .map(|v| v.len().to_string())
                    .unwrap_or_else(|| "0".to_string()),
                ParamExpansion::DefaultValue {
                    default,
                    assign_if_unset,
                } => match val {
                    Some(v) if !v.is_empty() => v,
                    _ => {
                        if *assign_if_unset {
                            env.set_var(name, default, false);
                        }
                        default.clone()
                    }
                },
                ParamExpansion::UseAlternative { alternative } => match val {
                    Some(v) if !v.is_empty() => alternative.clone(),
                    _ => String::new(),
                },
                ParamExpansion::ErrorIfUnset { message } => match val {
                    Some(v) if !v.is_empty() => v,
                    _ => {
                        let msg = if message.is_empty() {
                            format!("{}: parameter null or not set", name)
                        } else {
                            message.clone()
                        };
                        return Err(ShellError::ExpansionError(msg));
                    }
                },
                ParamExpansion::RemoveSuffix { pattern, longest } => {
                    let v = val.unwrap_or_default();
                    if *longest {
                        if let Some(idx) = v.rfind(pattern) {
                            v[..idx].to_string()
                        } else {
                            v
                        }
                    } else if let Some(idx) = v.find(pattern) {
                        v[..idx].to_string()
                    } else {
                        v
                    }
                }
                ParamExpansion::RemovePrefix { pattern, longest } => {
                    let v = val.unwrap_or_default();
                    if *longest {
                        if let Some(idx) = v.rfind(pattern) {
                            v[idx + pattern.len()..].to_string()
                        } else {
                            v
                        }
                    } else if let Some(idx) = v.find(pattern) {
                        v[idx + pattern.len()..].to_string()
                    } else {
                        v
                    }
                }
            };
            Ok((result_str, false))
        }
        WordPart::Arithmetic(expr) => {
            let val = eval_arithmetic(env, expr)?;
            Ok((val.to_string(), false))
        }
        WordPart::CommandSubstitution(cmd_str) => {
            let output = crate::exec::eval_command_substitution(env, cmd_str)?;
            // Trim trailing newlines per POSIX spec
            let trimmed = output.trim_end_matches('\n').to_string();
            Ok((trimmed, false))
        }
    }
}
