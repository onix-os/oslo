use crate::env::Environment;
use crate::error::{Result, ShellError};

pub fn eval_arithmetic(env: &Environment, expr: &str) -> Result<i64> {
    let expr = expr.trim();
    if expr.is_empty() {
        return Ok(0);
    }

    // Replace variable names in expr with their values
    let mut expanded = String::new();
    let mut word = String::new();

    for ch in expr.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            word.push(ch);
        } else {
            if !word.is_empty() {
                if let Ok(num) = word.parse::<i64>() {
                    expanded.push_str(&num.to_string());
                } else if let Some(val) = env.get_param(&word) {
                    let v = val.trim().parse::<i64>().unwrap_or(0);
                    expanded.push_str(&v.to_string());
                } else {
                    expanded.push('0');
                }
                word.clear();
            }
            expanded.push(ch);
        }
    }

    if !word.is_empty() {
        if let Ok(num) = word.parse::<i64>() {
            expanded.push_str(&num.to_string());
        } else if let Some(val) = env.get_param(&word) {
            let v = val.trim().parse::<i64>().unwrap_or(0);
            expanded.push_str(&v.to_string());
        } else {
            expanded.push('0');
        }
    }

    // Basic expression parser for +, -, *, /, %, (, )
    parse_expr(&mut expanded.chars().peekable())
}

fn parse_expr<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) -> Result<i64> {
    let mut left = parse_term(chars)?;

    while let Some(&op) = chars.peek() {
        if op == ' ' || op == '\t' {
            chars.next();
            continue;
        }

        if op == '+' || op == '-' {
            chars.next();
            let right = parse_term(chars)?;
            if op == '+' {
                left += right;
            } else {
                left -= right;
            }
        } else {
            break;
        }
    }

    Ok(left)
}

fn parse_term<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) -> Result<i64> {
    let mut left = parse_factor(chars)?;

    while let Some(&op) = chars.peek() {
        if op == ' ' || op == '\t' {
            chars.next();
            continue;
        }

        if op == '*' || op == '/' || op == '%' {
            chars.next();
            let right = parse_factor(chars)?;
            if op == '*' {
                left *= right;
            } else if op == '/' {
                if right == 0 {
                    return Err(ShellError::ExpansionError("Division by zero".to_string()));
                }
                left /= right;
            } else if op == '%' {
                if right == 0 {
                    return Err(ShellError::ExpansionError("Division by zero".to_string()));
                }
                left %= right;
            }
        } else {
            break;
        }
    }

    Ok(left)
}

fn parse_factor<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) -> Result<i64> {
    while let Some(&ch) = chars.peek() {
        if ch == ' ' || ch == '\t' {
            chars.next();
        } else {
            break;
        }
    }

    let ch = chars.peek().copied().ok_or_else(|| {
        ShellError::ExpansionError("Unexpected end of arithmetic expression".to_string())
    })?;

    if ch == '(' {
        chars.next(); // (
        let val = parse_expr(chars)?;
        while let Some(&c) = chars.peek() {
            if c == ' ' || c == '\t' {
                chars.next();
            } else {
                break;
            }
        }
        if chars.peek() == Some(&')') {
            chars.next(); // )
        }
        Ok(val)
    } else if ch == '+' {
        chars.next();
        parse_factor(chars)
    } else if ch == '-' {
        chars.next();
        Ok(-parse_factor(chars)?)
    } else if ch.is_ascii_digit() {
        let mut num_str = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() {
                num_str.push(c);
                chars.next();
            } else {
                break;
            }
        }
        num_str
            .parse::<i64>()
            .map_err(|e| ShellError::ExpansionError(e.to_string()))
    } else {
        Err(ShellError::ExpansionError(format!(
            "Invalid character in arithmetic expression: {}",
            ch
        )))
    }
}
