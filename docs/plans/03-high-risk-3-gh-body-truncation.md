# Implementation Plan: HIGH RISK 3 — gh Issue/PR Body Truncation

**Design Doc**: [03-high-risk-3-gh-body-truncation.md](../design-docs/03-high-risk-3-gh-body-truncation.md)
**Date**: 2026-02-22
**Issues**: [#188](https://github.com/rtk-ai/rtk/issues/188) (gh issue view — already fixed), [#199](https://github.com/rtk-ai/rtk/issues/199) (gh api)

**Safety invariants for ALL steps**:
- String truncation MUST use `s.chars().take(N)`, NEVER `&s[..N]` (panics on multi-byte UTF-8)
- JSON string output MUST use `serde_json::to_string()`, NEVER `format!("\"{}\"", s)` (malformed output)
- `timer.track()` MUST be called BEFORE `std::process::exit()` (preserves tracking data)
- `filter_json_compact()` MUST be applied to `stdout` only, NEVER concatenated `stdout+stderr` (breaks JSON parsing)

---

## Steps

### Step 1: Create test fixture for `gh api` JSON output

Create `tests/fixtures/gh_api_issues.json` with realistic GitHub API response data.

**File to create:**

```json
[
  {
    "number": 42,
    "title": "Fix login bug",
    "state": "open",
    "url": "https://github.com/org/repo/issues/42",
    "body": "The login form breaks when the password contains special characters like `<` and `>`.\n\nSteps to reproduce:\n1. Go to /login\n2. Enter password with `<script>` tag\n3. Submit\n\nExpected: Error message\nActual: 500 error",
    "labels": [{"name": "bug"}, {"name": "priority:high"}, {"name": "area:auth"}],
    "assignees": [{"login": "alice"}, {"login": "bob"}],
    "created_at": "2026-01-15T10:30:00Z",
    "updated_at": "2026-02-20T14:22:00Z"
  },
  {
    "number": 43,
    "title": "Add dark mode support",
    "state": "open",
    "url": "https://github.com/org/repo/issues/43",
    "body": "It would be great to have a dark mode toggle in the settings page.\n\nDesign mockup attached.",
    "labels": [{"name": "enhancement"}, {"name": "ui"}],
    "assignees": [],
    "created_at": "2026-01-16T08:00:00Z",
    "updated_at": "2026-01-16T08:00:00Z"
  },
  {
    "number": 44,
    "title": "Update README with installation instructions",
    "state": "closed",
    "url": "https://github.com/org/repo/issues/44",
    "body": "",
    "labels": [{"name": "docs"}],
    "assignees": [{"login": "charlie"}],
    "created_at": "2026-01-17T12:00:00Z",
    "updated_at": "2026-02-01T09:15:00Z"
  },
  {
    "number": 45,
    "title": "Performance regression in dashboard API",
    "state": "open",
    "url": "https://github.com/org/repo/issues/45",
    "body": "After upgrading to v2.3, the /api/dashboard endpoint takes 5 seconds to respond instead of 200ms.\n\nProfiler output shows N+1 query in the user stats aggregation.\n\n```sql\nSELECT * FROM user_stats WHERE user_id = ? -- called 500 times!\n```\n\nThis should be a single batch query:\n\n```sql\nSELECT * FROM user_stats WHERE user_id IN (?, ?, ...)\n```",
    "labels": [{"name": "bug"}, {"name": "performance"}],
    "assignees": [{"login": "alice"}],
    "created_at": "2026-02-10T16:45:00Z",
    "updated_at": "2026-02-22T11:00:00Z"
  }
]
```

This fixture covers: arrays of objects, nested arrays (labels, assignees), multi-line markdown bodies with code blocks, empty strings, URLs, dates.

**Run:** Verify file is valid JSON with `cat tests/fixtures/gh_api_issues.json | python3 -m json.tool > /dev/null`.

---

### Step 2: Create test fixture for `gh api` error response

Create `tests/fixtures/gh_api_error.json` with a typical GitHub API error.

**File to create:**

```json
{
  "message": "Validation Failed",
  "errors": [
    {
      "resource": "Issue",
      "code": "missing_field",
      "field": "title"
    }
  ],
  "documentation_url": "https://docs.github.com/rest/reference/issues#create-an-issue"
}
```

---

### Step 3: Write failing tests for `filter_json_compact()` (TDD Red)

Add tests to `src/json_cmd.rs` `mod tests` that call `filter_json_compact()` and assert correct behavior. These tests will FAIL because the function doesn't exist yet.

**Tests to add:**

```rust
#[test]
fn test_compact_preserves_values() {
    let input = r#"{"name": "test", "count": 42, "active": true}"#;
    let result = filter_json_compact(input, 5).unwrap();
    // Must contain actual values, not type schemas
    assert!(result.contains("\"test\""), "Must preserve string value");
    assert!(result.contains("42"), "Must preserve number value");
    assert!(result.contains("true"), "Must preserve boolean value");
    // Must NOT contain type schemas
    assert!(!result.contains(": string"), "Must not show type schema");
    assert!(!result.contains(": int"), "Must not show type schema");
}

#[test]
fn test_compact_truncates_long_strings() {
    let long_str = "a".repeat(300);
    let input = format!(r#"{{"body": "{}"}}"#, long_str);
    let result = filter_json_compact(&input, 5).unwrap();
    // Must truncate but show length
    assert!(result.contains("...[300 chars]"), "Must show char count");
    // Must NOT contain the full 300-char string
    assert!(!result.contains(&long_str), "Must truncate long string");
}

#[test]
fn test_compact_unicode_safe() {
    // 99 ASCII chars + emoji (4 bytes) = 103 bytes, >100 byte boundary
    let s = format!("{}🚀 more text after emoji", "a".repeat(99));
    let input = format!(r#"{{"body": "{}"}}"#, s);
    // Must NOT panic on multi-byte char boundary
    let result = filter_json_compact(&input, 5).unwrap();
    assert!(result.contains("..."));
}

#[test]
fn test_compact_escapes_strings() {
    let input = r#"{"msg": "line1\nline2", "quote": "he said \"hello\""}"#;
    let result = filter_json_compact(input, 5).unwrap();
    // Must contain properly escaped strings (no raw newlines breaking output)
    assert!(result.contains("msg"));
    assert!(result.contains("quote"));
}

#[test]
fn test_compact_collapses_large_arrays() {
    let input = r#"{"items": [1, 2, 3, 4, 5, 6, 7]}"#;
    let result = filter_json_compact(input, 5).unwrap();
    // Must show first 3 + count
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
    // At depth 3, deeper objects should collapse
    assert!(result.contains("..."), "Must collapse beyond max_depth");
}

#[test]
fn test_compact_gh_api_fixture() {
    let input = include_str!("../tests/fixtures/gh_api_issues.json");
    let result = filter_json_compact(input, 5).unwrap();
    // Must preserve actual issue data
    assert!(result.contains("Fix login bug"), "Must preserve title");
    assert!(result.contains("42"), "Must preserve issue number");
    assert!(result.contains("github.com"), "Must preserve URL");
    // Must preserve labels
    assert!(result.contains("bug"), "Must preserve label name");
}

#[test]
fn test_compact_gh_api_error() {
    let input = include_str!("../tests/fixtures/gh_api_error.json");
    let result = filter_json_compact(input, 5).unwrap();
    // Must preserve actual error details
    assert!(result.contains("Validation Failed"), "Must preserve error message");
    assert!(result.contains("missing_field"), "Must preserve error code");
    assert!(result.contains("title"), "Must preserve field name");
}

#[test]
fn test_schema_unchanged() {
    // Verify filter_json_string still works as before (regression check)
    let input = r#"{"name": "test", "count": 42}"#;
    let result = filter_json_string(input, 5).unwrap();
    assert!(result.contains("string"), "Schema must show types");
    assert!(result.contains("int"), "Schema must show types");
    assert!(!result.contains("test"), "Schema must NOT show values");
}
```

**Run:** `cargo test json_cmd::tests` — expect compile errors (function doesn't exist yet).

---

### Step 4: Implement `filter_json_compact()` and `compact_json()` in `json_cmd.rs`

Add the two new functions after `filter_json_string()` (after current line 55), before `fn extract_schema()`.

**Code to add after line 55:**

```rust
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
```

**Run:** `cargo test json_cmd::tests` — all tests should pass (TDD Green).

---

### Step 5: Rewrite `run_api()` in `gh_cmd.rs`

Replace the entire `run_api()` function (lines 1121-1163) with the corrected version that:
1. Removes the early exit on failure (which dropped stdout)
2. Uses `filter_json_compact()` instead of `filter_json_string()`
3. Processes stdout and stderr separately
4. Follows the Risk 2 invariant: `println` → `timer.track` → `process::exit`
5. Increases non-JSON line limit from 20 to 100

**Replace lines 1121-1163 with:**

```rust
fn run_api(args: &[String], _verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    let mut cmd = Command::new("gh");
    cmd.arg("api");
    for arg in args {
        cmd.arg(arg);
    }

    let output = cmd.output().context("Failed to run gh api")?;

    // Process stdout and stderr SEPARATELY (mixing breaks JSON parsing)
    let raw_stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let raw_stderr = String::from_utf8_lossy(&output.stderr).to_string();

    // Apply compact filter to stdout only
    let filtered_stdout = match json_cmd::filter_json_compact(&raw_stdout, 5) {
        Ok(compacted) => compacted,
        Err(_) => {
            // Not JSON, show truncated raw stdout (100 lines)
            let lines: Vec<&str> = raw_stdout.lines().take(100).collect();
            let mut result = lines.join("\n");
            if raw_stdout.lines().count() > 100 {
                result.push_str("\n... (truncated)");
            }
            result
        }
    };

    // 1. Print to user
    if !filtered_stdout.is_empty() {
        println!("{}", filtered_stdout);
    }
    if !raw_stderr.trim().is_empty() {
        eprintln!("{}", raw_stderr.trim());
    }

    // 2. Track BEFORE exit (Risk 2 invariant)
    let raw_combined = format!("{}\n{}", raw_stdout, raw_stderr);
    let rtk_combined = format!("{}\n{}", filtered_stdout, raw_stderr);
    timer.track("gh api", "rtk gh api", &raw_combined, &rtk_combined);

    // 3. Exit code propagation is LAST (Risk 2 invariant)
    if !output.status.success() {
        std::process::exit(output.status.code().unwrap_or(1));
    }

    Ok(())
}
```

**Run:** `cargo test gh_cmd::tests` — passes.

---

### Step 6: Run full quality gate

```bash
cargo fmt --all --check
cargo clippy --all-targets
cargo test --all
```

All three must pass with zero warnings. Pay special attention to:
- No unused imports (if `json_cmd::filter_json_string` is no longer used in `gh_cmd.rs`, it's still used by `json_cmd::run()` — safe)
- No dead code warnings from the new functions

---

### Step 7: Manual verification

Test with real `gh` commands (if `gh` CLI is installed and authenticated):

```bash
cargo build

# gh api: should show actual values, not type schemas
target/debug/rtk gh api repos/rtk-ai/rtk/issues?per_page=2
# Expected: actual issue titles, numbers, URLs visible

# gh api: exit code on error
target/debug/rtk gh api repos/nonexistent/nonexistent; echo "exit: $?"
# Expected: error JSON visible in stdout, exit: 1

# gh api: non-JSON output
target/debug/rtk gh api repos/rtk-ai/rtk/readme --header "Accept: application/vnd.github.raw"; echo "exit: $?"
# Expected: up to 100 lines of raw output

# Regression: gh issue view still works
target/debug/rtk gh issue view 1 --repo rtk-ai/rtk
# Expected: full body shown

# Regression: gh pr view still works
target/debug/rtk gh pr view 1 --repo rtk-ai/rtk
# Expected: full body shown

# Verify tracking works
target/debug/rtk gh api repos/rtk-ai/rtk/issues?per_page=2
target/debug/rtk gain --history | tail -5
# Expected: gh api appears in history with savings recorded
```

If `gh` is not installed, rely on unit tests from Steps 3-4.

---

### Step 8: Commit

Single commit with all changes:
- `src/json_cmd.rs` — `filter_json_compact()` + `compact_json()` + tests
- `src/gh_cmd.rs` — rewritten `run_api()` with Risk 2 invariant compliance
- `tests/fixtures/gh_api_issues.json` — new fixture
- `tests/fixtures/gh_api_error.json` — new fixture

---

## Summary

| Step | What | File | Validates |
|------|------|------|-----------|
| 1 | Create gh api issues fixture | `tests/fixtures/` | Real API response data |
| 2 | Create gh api error fixture | `tests/fixtures/` | Error response data |
| 3 | Write failing tests (TDD Red) | `json_cmd.rs` | Bug confirmed, Unicode safety, escaping |
| 4 | Implement `filter_json_compact()` (TDD Green) | `json_cmd.rs` | Values preserved, strings safe |
| 5 | Rewrite `run_api()` | `gh_cmd.rs` | Risk 2 invariant, stdout on failure, 100-line limit |
| 6 | Full quality gate | all | fmt + clippy + test = clean |
| 7 | Manual verification | binary | End-to-end with real `gh` |
| 8 | Commit | all | All changes captured |

**Safety checks in every step**:
- `s.chars().take(N)` for string truncation (never `&s[..N]`)
- `serde_json::to_string()` for JSON strings (never `format!("\"{}\"")`)
- `timer.track()` always precedes `process::exit()`
- `filter_json_compact()` applied to `stdout` only (never `stdout+stderr`)
