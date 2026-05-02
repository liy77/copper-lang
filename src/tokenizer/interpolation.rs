//! String-interpolation parser.
//!
//! Copper string literals support two interpolation forms, identical to the
//! ones documented in the language README:
//!
//! ```text
//! "Hello $name"            // bare identifier
//! "Total: ${count * 2}"    // braced expression
//! ```
//!
//! [`parse`] walks a quoted literal once and produces an [`Interpolated`]
//! description: a format string with `{}` placeholders plus the list of Rust
//! expressions to splice in. Returns `None` for plain strings, so the caller
//! can leave them untouched.
//!
//! Conventions worth knowing:
//! * `\$` is the escape for a literal `$` — it survives as `$` in the output.
//!   (Plain `\$` in a Rust string literal would be a parse error, so the
//!   tokenizer must rewrite it.)
//! * `{` and `}` in the source string become `{{` / `}}` in the format
//!   string, but only when interpolation is present (otherwise the original
//!   literal is returned unchanged).
//! * `${...}` tracks brace depth, so nested `{}` inside the expression
//!   work — e.g. `${ vec![1,2,3].len() }`.

/// Result of parsing an interpolated literal.
#[derive(Debug, Clone, PartialEq)]
pub struct Interpolated {
    /// The format string, with `{}` markers where each arg should land.
    /// Already escaped: literal `{`/`}` are doubled, and `\$` is collapsed
    /// to `$`. Does **not** include surrounding double quotes.
    pub placeholder: String,
    /// Expressions that fill the placeholders, in order.
    pub args: Vec<String>,
}

/// Parse a quoted string literal (with surrounding `"` characters) and return
/// an [`Interpolated`] description if it contains `$ident` or `${expr}`. Plain
/// strings — no interpolation, or only escaped `$` — return `None`.
pub fn parse(quoted: &str) -> Option<Interpolated> {
    let body = strip_quotes(quoted)?;

    let mut placeholder = String::new();
    let mut args: Vec<String> = Vec::new();
    let mut found_interp = false;

    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                // Preserve escape sequences. The one we own is `\$` → literal
                // `$` (Rust strings reject `\$`, so we must collapse it).
                match chars.peek() {
                    Some('$') => {
                        chars.next();
                        placeholder.push('$');
                    }
                    Some(&next) => {
                        placeholder.push('\\');
                        placeholder.push(next);
                        chars.next();
                    }
                    None => placeholder.push('\\'),
                }
            }
            '$' => match chars.peek() {
                Some('{') => {
                    chars.next(); // consume `{`
                    let expr = read_braced_expression(&mut chars);
                    placeholder.push_str("{}");
                    args.push(expr);
                    found_interp = true;
                }
                Some(&next) if is_ident_start(next) => {
                    let ident = read_ident(&mut chars);
                    placeholder.push_str("{}");
                    args.push(ident);
                    found_interp = true;
                }
                _ => placeholder.push('$'),
            },
            // Brace literals must be doubled for `format!` once we know the
            // string is becoming a format string. We always double here and
            // discard the work below if no interpolation was found.
            '{' => placeholder.push_str("{{"),
            '}' => placeholder.push_str("}}"),
            _ => placeholder.push(c),
        }
    }

    if !found_interp {
        return None;
    }
    Some(Interpolated { placeholder, args })
}

/// Render the interpolated literal as a `format!(...)` call. This is the
/// default expansion — works in any expression position.
pub fn render_format_call(interp: &Interpolated) -> String {
    let mut out = String::with_capacity(interp.placeholder.len() + 16);
    out.push_str("format!(\"");
    out.push_str(&interp.placeholder);
    out.push('"');
    for arg in &interp.args {
        out.push_str(", ");
        out.push_str(arg);
    }
    out.push(')');
    out
}

/// Render the interpolated literal as raw `"placeholder", args...` — suitable
/// for splicing directly into the argument list of a `name!(...)` macro call
/// that already takes a format string (e.g. `println!`, `format!`, `write!`).
pub fn render_macro_args(interp: &Interpolated) -> String {
    let mut out = String::with_capacity(interp.placeholder.len() + 8);
    out.push('"');
    out.push_str(&interp.placeholder);
    out.push('"');
    for arg in &interp.args {
        out.push_str(", ");
        out.push_str(arg);
    }
    out
}

// -- helpers ----------------------------------------------------------------

fn strip_quotes(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'"' || bytes[bytes.len() - 1] != b'"' {
        return None;
    }
    Some(&s[1..s.len() - 1])
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn read_ident(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut out = String::new();
    while let Some(&c) = chars.peek() {
        if is_ident_continue(c) {
            out.push(c);
            chars.next();
        } else {
            break;
        }
    }
    out
}

/// Read an expression inside `${...}` until the matching `}`, tracking nested
/// braces. The opening `{` should already be consumed.
fn read_braced_expression(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut out = String::new();
    let mut depth: usize = 1;
    for c in chars.by_ref() {
        match c {
            '{' => {
                depth += 1;
                out.push(c);
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_string_is_none() {
        assert!(parse("\"hello world\"").is_none());
    }

    #[test]
    fn bare_identifier() {
        let i = parse("\"Hello $name\"").unwrap();
        assert_eq!(i.placeholder, "Hello {}");
        assert_eq!(i.args, vec!["name".to_string()]);
    }

    #[test]
    fn braced_expression() {
        let i = parse("\"Total: ${count * 2}\"").unwrap();
        assert_eq!(i.placeholder, "Total: {}");
        assert_eq!(i.args, vec!["count * 2".to_string()]);
    }

    #[test]
    fn escaped_dollar_is_literal() {
        assert!(parse("\"price: \\$5\"").is_none());
    }

    #[test]
    fn double_braces_are_escaped_when_interpolating() {
        let i = parse("\"json: {\\\"k\\\": $v}\"").unwrap();
        assert!(i.placeholder.contains("{{"));
        assert!(i.placeholder.contains("}}"));
    }

    #[test]
    fn nested_braces_in_expression() {
        let i = parse("\"${ vec![1,2,3].iter().sum::<i32>() }\"").unwrap();
        assert_eq!(i.placeholder, "{}");
        assert_eq!(i.args, vec!["vec![1,2,3].iter().sum::<i32>()".to_string()]);
    }
}
