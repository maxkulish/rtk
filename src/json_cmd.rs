use crate::tracking;
use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

/// Show JSON structure without values
pub fn run(file: &Path, max_depth: usize, verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    if verbose > 0 {
        eprintln!("Analyzing JSON: {}", file.display());
    }

    let content = fs::read_to_string(file)
        .with_context(|| format!("Failed to read file: {}", file.display()))?;

    let schema = filter_json_string(&content, max_depth)?;
    println!("{}", schema);
    timer.track(
        &format!("cat {}", file.display()),
        "rtk json",
        &content,
        &schema,
    );
    Ok(())
}

/// Show JSON structure from stdin
pub fn run_stdin(max_depth: usize, verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    if verbose > 0 {
        eprintln!("Analyzing JSON from stdin");
    }

    let mut content = String::new();
    io::stdin()
        .lock()
        .read_to_string(&mut content)
        .context("Failed to read from stdin")?;

    let schema = filter_json_string(&content, max_depth)?;
    println!("{}", schema);
    timer.track("cat - (stdin)", "rtk json -", &content, &schema);
    Ok(())
}

/// Parse a JSON string and return its schema representation.
/// Useful for piping JSON from other commands (e.g., `gh api`, `curl`).
pub fn filter_json_string(json_str: &str, max_depth: usize) -> Result<String> {
    let value: Value = serde_json::from_str(json_str).context("Failed to parse JSON")?;
    Ok(extract_schema(&value, 0, max_depth))
}

/// Compact JSON for API output: preserves actual values, truncates long strings,
/// collapses large arrays. Unlike filter_json_string() which shows schema only.
pub fn filter_json_compact(json_str: &str, max_depth: usize) -> Result<String> {
    let value: Value = serde_json::from_str(json_str).context("Failed to parse JSON")?;
    Ok(compact_json(&value, 0, max_depth))
}

fn compact_json(value: &Value, depth: usize, max_depth: usize) -> String {
    if depth > max_depth {
        return match value {
            Value::Object(map) => format!("{{...{} keys}}", map.len()),
            Value::Array(arr) => format!("[...{} items]", arr.len()),
            _ => value.to_string(),
        };
    }

    match value {
        Value::String(s) => {
            if s.len() > 200 {
                let truncated: String = s.chars().take(100).collect();
                let display = format!("{}...[{} chars]", truncated, s.chars().count());
                serde_json::to_string(&display).unwrap_or_else(|_| "\"Error\"".to_string())
            } else {
                serde_json::to_string(s).unwrap_or_else(|_| "\"Error\"".to_string())
            }
        }
        Value::Array(arr) => {
            if arr.is_empty() {
                "[]".to_string()
            } else if arr.len() <= 3 {
                let items: Vec<String> = arr
                    .iter()
                    .map(|v| compact_json(v, depth + 1, max_depth))
                    .collect();
                format!("[{}]", items.join(", "))
            } else {
                let items: Vec<String> = arr
                    .iter()
                    .take(3)
                    .map(|v| compact_json(v, depth + 1, max_depth))
                    .collect();
                format!("[{}, ...+{} more]", items.join(", "), arr.len() - 3)
            }
        }
        Value::Object(map) => {
            if map.is_empty() {
                return "{}".to_string();
            }
            let indent = "  ".repeat(depth + 1);
            let close_indent = "  ".repeat(depth);
            let mut lines = vec!["{".to_string()];
            let keys: Vec<_> = map.keys().collect();
            let show = keys.len().min(20);

            for (i, key) in keys.iter().take(show).enumerate() {
                let val = compact_json(&map[*key], depth + 1, max_depth);
                let is_last_shown = i == show - 1;
                let has_more = keys.len() > 20;
                let comma = if is_last_shown && !has_more { "" } else { "," };
                lines.push(format!("{}\"{}\": {}{}", indent, key, val, comma));
            }
            if keys.len() > 20 {
                lines.push(format!("{}...+{} more keys", indent, keys.len() - 20));
            }
            lines.push(format!("{}}}", close_indent));
            lines.join("\n")
        }
        _ => value.to_string(),
    }
}

fn extract_schema(value: &Value, depth: usize, max_depth: usize) -> String {
    let indent = "  ".repeat(depth);

    if depth > max_depth {
        return format!("{}...", indent);
    }

    match value {
        Value::Null => format!("{}null", indent),
        Value::Bool(_) => format!("{}bool", indent),
        Value::Number(n) => {
            if n.is_i64() {
                format!("{}int", indent)
            } else {
                format!("{}float", indent)
            }
        }
        Value::String(s) => {
            if s.len() > 50 {
                format!("{}string[{}]", indent, s.len())
            } else if s.is_empty() {
                format!("{}string", indent)
            } else {
                // Check if it looks like a URL, date, etc.
                if s.starts_with("http") {
                    format!("{}url", indent)
                } else if s.contains('-') && s.len() == 10 {
                    format!("{}date?", indent)
                } else {
                    format!("{}string", indent)
                }
            }
        }
        Value::Array(arr) => {
            if arr.is_empty() {
                format!("{}[]", indent)
            } else {
                let first_schema = extract_schema(&arr[0], depth + 1, max_depth);
                let trimmed = first_schema.trim();
                if arr.len() == 1 {
                    format!("{}[\n{}\n{}]", indent, first_schema, indent)
                } else {
                    format!("{}[{}] ({})", indent, trimmed, arr.len())
                }
            }
        }
        Value::Object(map) => {
            if map.is_empty() {
                format!("{}{{}}", indent)
            } else {
                let mut lines = vec![format!("{}{{", indent)];
                let mut keys: Vec<_> = map.keys().collect();
                keys.sort();

                for (i, key) in keys.iter().enumerate() {
                    let val = &map[*key];
                    let val_schema = extract_schema(val, depth + 1, max_depth);
                    let val_trimmed = val_schema.trim();

                    // Inline simple types
                    let is_simple = matches!(
                        val,
                        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
                    );

                    if is_simple {
                        if i < keys.len() - 1 {
                            lines.push(format!("{}  {}: {},", indent, key, val_trimmed));
                        } else {
                            lines.push(format!("{}  {}: {}", indent, key, val_trimmed));
                        }
                    } else {
                        lines.push(format!("{}  {}:", indent, key));
                        lines.push(val_schema);
                    }

                    // Limit keys shown
                    if i >= 15 {
                        lines.push(format!("{}  ... +{} more keys", indent, keys.len() - i - 1));
                        break;
                    }
                }
                lines.push(format!("{}}}", indent));
                lines.join("\n")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_schema_simple() {
        let json: Value = serde_json::from_str(r#"{"name": "test", "count": 42}"#).unwrap();
        let schema = extract_schema(&json, 0, 5);
        assert!(schema.contains("name"));
        assert!(schema.contains("string"));
        assert!(schema.contains("int"));
    }

    #[test]
    fn test_extract_schema_array() {
        let json: Value = serde_json::from_str(r#"{"items": [1, 2, 3]}"#).unwrap();
        let schema = extract_schema(&json, 0, 5);
        assert!(schema.contains("items"));
        assert!(schema.contains("(3)"));
    }

    #[test]
    fn test_compact_preserves_values() {
        let input = r#"{"name": "test", "count": 42, "active": true}"#;
        let result = filter_json_compact(input, 5).unwrap();
        assert!(result.contains("\"test\""), "Must preserve string value");
        assert!(result.contains("42"), "Must preserve number value");
        assert!(result.contains("true"), "Must preserve boolean value");
        assert!(!result.contains(": string"), "Must not show type schema");
        assert!(!result.contains(": int"), "Must not show type schema");
    }

    #[test]
    fn test_compact_truncates_long_strings() {
        let long_str = "a".repeat(300);
        let input = format!(r#"{{"body": "{}"}}"#, long_str);
        let result = filter_json_compact(&input, 5).unwrap();
        assert!(result.contains("...[300 chars]"), "Must show char count");
        assert!(!result.contains(&long_str), "Must truncate long string");
    }

    #[test]
    fn test_compact_unicode_safe() {
        // 199 ASCII chars + emoji (4 bytes) = 203 bytes, >200 byte boundary
        // Naive &s[..200] would panic (byte 200 falls inside the 4-byte emoji)
        // s.chars().take(100) must be used instead
        let s = format!("{}🚀 more text after emoji", "a".repeat(199));
        let input = format!(r#"{{"body": "{}"}}"#, s);
        // Must NOT panic on multi-byte char boundary
        let result = filter_json_compact(&input, 5).unwrap();
        assert!(result.contains("..."));
    }

    #[test]
    fn test_compact_escapes_strings() {
        let input = r#"{"msg": "line1\nline2", "quote": "he said \"hello\""}"#;
        let result = filter_json_compact(input, 5).unwrap();
        assert!(result.contains("msg"));
        assert!(result.contains("quote"));
    }

    #[test]
    fn test_compact_collapses_large_arrays() {
        let input = r#"{"items": [1, 2, 3, 4, 5, 6, 7]}"#;
        let result = filter_json_compact(input, 5).unwrap();
        assert!(result.contains("1"), "Must show first element");
        assert!(result.contains("2"), "Must show second element");
        assert!(result.contains("3"), "Must show third element");
        assert!(result.contains("+4 more"), "Must show remaining count");
    }

    #[test]
    fn test_compact_small_arrays_shown_fully() {
        let input = r#"{"items": [1, 2, 3]}"#;
        let result = filter_json_compact(input, 5).unwrap();
        assert!(result.contains("1"));
        assert!(result.contains("2"));
        assert!(result.contains("3"));
        assert!(!result.contains("more"), "Small arrays shown in full");
    }

    #[test]
    fn test_compact_depth_limit() {
        let input = r#"{"a": {"b": {"c": {"d": {"e": {"f": "deep"}}}}}}"#;
        let result = filter_json_compact(input, 3).unwrap();
        assert!(result.contains("..."), "Must collapse beyond max_depth");
    }

    #[test]
    fn test_compact_gh_api_fixture() {
        let input = include_str!("../tests/fixtures/gh_api_issues.json");
        let result = filter_json_compact(input, 5).unwrap();
        assert!(result.contains("Fix login bug"), "Must preserve title");
        assert!(result.contains("42"), "Must preserve issue number");
        assert!(result.contains("github.com"), "Must preserve URL");
        assert!(result.contains("bug"), "Must preserve label name");
    }

    #[test]
    fn test_compact_gh_api_error() {
        let input = include_str!("../tests/fixtures/gh_api_error.json");
        let result = filter_json_compact(input, 5).unwrap();
        assert!(
            result.contains("Validation Failed"),
            "Must preserve error message"
        );
        assert!(result.contains("missing_field"), "Must preserve error code");
        assert!(result.contains("title"), "Must preserve field name");
    }

    #[test]
    fn test_schema_unchanged() {
        let input = r#"{"name": "test", "count": 42}"#;
        let result = filter_json_string(input, 5).unwrap();
        assert!(result.contains("string"), "Schema must show types");
        assert!(result.contains("int"), "Schema must show types");
        assert!(!result.contains("\"test\""), "Schema must NOT show values");
    }
}
