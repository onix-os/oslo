use crate::env::Environment;
use crate::error::{Result, ShellError};

/// Deepest parenthesis (or unary-operator) nesting an arithmetic expression may use.
///
/// The expression grammar is recursive descent, so `$(( ((((1)))) ))` costs a stack frame per
/// parenthesis and `$(( ----1 ))` one per sign; 50 000 of either overflowed the stack and aborted
/// the shell. 100 is far past anything a human writes and far short of what the stack can take.
const MAX_DEPTH: usize = 100;

fn too_deep() -> ShellError {
    ShellError::ExecutionError("maximum nesting level exceeded".to_string())
}

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
                push_word(env, &word, &mut expanded);
                word.clear();
            }
            expanded.push(ch);
        }
    }

    if !word.is_empty() {
        push_word(env, &word, &mut expanded);
    }

    // Basic expression parser for +, -, *, /, %, (, )
    parse_expr(&mut expanded.chars().peekable(), 0)
}

/// Substitute one operand word into the expression text.
///
/// A run of digits is copied verbatim rather than round-tripped through `i64`: a literal wider
/// than `i64` must reach the parser so it can wrap like bash, and the old parse-or-zero path
/// silently turned `9223372036854775808` (the magnitude of `i64::MIN`) into `0`.
fn push_word(env: &Environment, word: &str, out: &mut String) {
    if word.bytes().all(|b| b.is_ascii_digit()) {
        out.push_str(word);
    } else if let Some(val) = env.get_param(word) {
        let v = val.trim().parse::<i64>().unwrap_or(0);
        out.push_str(&v.to_string());
    } else {
        out.push('0');
    }
}

/// `depth` is the number of enclosing parentheses and unary operators, capped at `MAX_DEPTH`.
fn parse_expr<I: Iterator<Item = char>>(
    chars: &mut std::iter::Peekable<I>,
    depth: usize,
) -> Result<i64> {
    let mut left = parse_term(chars, depth)?;

    while let Some(&op) = chars.peek() {
        if op == ' ' || op == '\t' {
            chars.next();
            continue;
        }

        if op == '+' || op == '-' {
            chars.next();
            let right = parse_term(chars, depth)?;
            // Wrapping, not checked: bash's arithmetic is C `intmax_t` arithmetic, which wraps.
            // Plain `+`/`-` would panic in a debug build and wrap in a release one — the same
            // input must not depend on the build profile.
            if op == '+' {
                left = left.wrapping_add(right);
            } else {
                left = left.wrapping_sub(right);
            }
        } else {
            break;
        }
    }

    Ok(left)
}

fn parse_term<I: Iterator<Item = char>>(
    chars: &mut std::iter::Peekable<I>,
    depth: usize,
) -> Result<i64> {
    let mut left = parse_factor(chars, depth)?;

    while let Some(&op) = chars.peek() {
        if op == ' ' || op == '\t' {
            chars.next();
            continue;
        }

        if op == '*' || op == '/' || op == '%' {
            chars.next();
            let right = parse_factor(chars, depth)?;
            if op == '*' {
                left = left.wrapping_mul(right);
            } else if op == '/' {
                if right == 0 {
                    return Err(ShellError::ExpansionError("Division by zero".to_string()));
                }
                // `i64::MIN / -1` is the one non-zero divisor that overflows; bash yields
                // `i64::MIN` for it, which is what the checked failure wraps to.
                left = left.checked_div(right).unwrap_or(i64::MIN);
            } else if op == '%' {
                if right == 0 {
                    return Err(ShellError::ExpansionError("Division by zero".to_string()));
                }
                // Same overflow case; the mathematical remainder is 0 and bash agrees.
                left = left.checked_rem(right).unwrap_or(0);
            }
        } else {
            break;
        }
    }

    Ok(left)
}

fn parse_factor<I: Iterator<Item = char>>(
    chars: &mut std::iter::Peekable<I>,
    depth: usize,
) -> Result<i64> {
    if depth > MAX_DEPTH {
        return Err(too_deep());
    }

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
        let val = parse_expr(chars, depth + 1)?;
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
        parse_factor(chars, depth + 1)
    } else if ch == '-' {
        chars.next();
        // `-i64::MIN` overflows, and `$((-9223372036854775808))` is how `i64::MIN` is written.
        Ok(parse_factor(chars, depth + 1)?.wrapping_neg())
    } else if ch.is_ascii_digit() {
        // Accumulated as `u64` so the magnitude of `i64::MIN` is representable; anything wider
        // wraps rather than erroring, matching C integer arithmetic.
        let mut value: u64 = 0;
        while let Some(&c) = chars.peek() {
            if let Some(digit) = c.to_digit(10) {
                value = value.wrapping_mul(10).wrapping_add(u64::from(digit));
                chars.next();
            } else {
                break;
            }
        }
        Ok(value as i64)
    } else {
        Err(ShellError::ExpansionError(format!(
            "Invalid character in arithmetic expression: {}",
            ch
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(expr: &str) -> i64 {
        let env = Environment::new();
        eval_arithmetic(&env, expr).expect("expression should evaluate")
    }

    fn eval_with(name: &str, value: &str, expr: &str) -> i64 {
        let mut env = Environment::new();
        env.set_var(name, value, false);
        eval_arithmetic(&env, expr).expect("expression should evaluate")
    }

    #[test]
    fn arithmetic_still_computes_ordinary_values() {
        assert_eq!(eval("1 + 2 * 3"), 7);
        assert_eq!(eval("(1 + 2) * 3"), 9);
        assert_eq!(eval("7 / 2"), 3);
        assert_eq!(eval("7 % 2"), 1);
        assert_eq!(eval("-3 + 1"), -2);
    }

    // Every case below panicked in a debug build and wrapped silently in a release one.

    #[test]
    fn addition_overflow_wraps_like_bash() {
        assert_eq!(eval("9223372036854775807 + 1"), i64::MIN);
        assert_eq!(eval("9223372036854775807 + 9223372036854775807"), -2);
    }

    #[test]
    fn subtraction_underflow_wraps_like_bash() {
        assert_eq!(eval("-9223372036854775807 - 2"), i64::MAX);
        assert_eq!(eval_with("a", "-9223372036854775808", "a - 1"), i64::MAX);
    }

    #[test]
    fn multiplication_overflow_wraps_like_bash() {
        assert_eq!(eval("9223372036854775807 * 2"), -2);
        assert_eq!(eval_with("a", "-9223372036854775808", "a * -1"), i64::MIN);
    }

    #[test]
    fn min_divided_by_negative_one_wraps_instead_of_panicking() {
        assert_eq!(eval_with("a", "-9223372036854775808", "a / -1"), i64::MIN);
        assert_eq!(eval("-9223372036854775808 / -1"), i64::MIN);
    }

    #[test]
    fn min_modulo_negative_one_is_zero() {
        assert_eq!(eval_with("a", "-9223372036854775808", "a % -1"), 0);
        assert_eq!(eval("-9223372036854775808 % -1"), 0);
    }

    #[test]
    fn min_literal_negates_without_overflow() {
        assert_eq!(eval("-9223372036854775808"), i64::MIN);
        assert_eq!(eval("- -9223372036854775808"), i64::MIN);
    }

    #[test]
    fn out_of_range_literal_wraps_rather_than_erroring() {
        // Previously this word never reached the parser: the operand pass turned any literal it
        // could not fit in an i64 into 0.
        assert_eq!(eval("9223372036854775808"), i64::MIN);
        assert_eq!(eval("18446744073709551616"), 0);
    }

    /// 50 000 parentheses used to recurse until the stack overflowed and the process aborted.
    #[test]
    fn absurd_parenthesis_nesting_is_an_error_not_a_crash() {
        let env = Environment::new();
        let expr = format!("{}1{}", "(".repeat(50_000), ")".repeat(50_000));
        let err = eval_arithmetic(&env, &expr).expect_err("must be refused");
        assert!(
            err.to_string().contains("maximum nesting level exceeded"),
            "{err}"
        );
    }

    /// Unary operators recurse through `parse_factor` just as parentheses do.
    #[test]
    fn absurd_unary_nesting_is_an_error_not_a_crash() {
        let env = Environment::new();
        let expr = format!("{}1", "-".repeat(50_000));
        assert!(eval_arithmetic(&env, &expr).is_err());
    }

    /// The limit has to leave ordinary — even unusually parenthesised — expressions alone.
    #[test]
    fn moderate_nesting_still_evaluates() {
        let expr = format!("{}1 + 1{}", "(".repeat(50), ")".repeat(50));
        assert_eq!(eval(&expr), 2);
    }

    #[test]
    fn division_by_zero_is_still_an_error() {
        let env = Environment::new();
        assert!(eval_arithmetic(&env, "1 / 0").is_err());
        assert!(eval_arithmetic(&env, "1 % 0").is_err());
    }
}
