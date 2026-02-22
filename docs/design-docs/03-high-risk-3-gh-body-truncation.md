# Design Document: HIGH RISK 3 — gh Issue/PR Body Truncation

**Date**: 2026-02-22
**Risk Level**: HIGH → proposed LOW (after analysis)
**Status**: Revised (v2) — incorporating review feedback
**Related Issues**: [#188](https://github.com/rtk-ai/rtk/issues/188) (gh issue view), [#199](https://github.com/rtk-ai/rtk/issues/199) (gh api)

---

## 1. Problem Statement

RTK wraps `gh` CLI commands and applies token-saving filters. Two issues reported that these filters destroyed critical data:

- **Issue #188**: `rtk gh issue view` truncated issue bodies to ~3 lines, making full issue text unreadable
- **Issue #199**: `rtk gh api` converts JSON values to type schemas (losing actual data) and truncates non-JSON output to 20 lines

### Current State After Codebase Audit

**Issue #188 — FIXED** (v0.22.1)

Both `view_issue()` (line 605-690) and `view_pr()` (line 229-396) now display the full body through `filter_markdown_body()`. This function:

- Preserves all text content and code blocks
- Strips only noise: HTML comments, badge lines, image-only lines, horizontal rules
- Collapses excessive blank lines
- Indents body content under a "Description:" header

The old `.take(3)` truncation is completely gone. The fix is correct and well-implemented.

**Issue #199 — PARTIALLY ADDRESSED**

`run_api()` (line 1121-1163) has three remaining problems:

**Problem A: JSON value destruction** — When `gh api` returns JSON, RTK applies `json_cmd::filter_json_string()` which replaces all values with type schemas:

```
// Input (actual gh api output):
{"title": "Fix bug", "number": 42, "url": "https://github.com/org/repo/issues/42"}

// RTK output (all data lost):
{
  number: int,
  title: string,
  url: url
}
```

This is the correct behavior for `rtk json` (exploratory schema inspection), but destructive for `rtk gh api` where the caller needs **actual values** for programmatic use (e.g., extracting issue numbers, URLs, labels for automation).

**Problem B: Non-JSON truncation** — When `gh api` returns non-JSON (e.g., raw text, GraphQL errors), RTK truncates to 20 lines with `"... (truncated)"`:

```rust
let lines: Vec<&str> = raw.lines().take(20).collect();
// ...
if raw.lines().count() > 20 {
    result.push_str("\n... (truncated)");
}
```

**Problem C: Early exit drops stdout on failure** — When `gh api` fails (non-zero exit), `run_api()` exits early, completely dropping `stdout`:

```rust
if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    timer.track("gh api", "rtk gh api", &stderr, &stderr);
    eprintln!("{}", stderr.trim());
    std::process::exit(output.status.code().unwrap_or(1));
}
```

GitHub API errors (HTTP 404, 422, etc.) typically return a JSON error payload in `stdout` (e.g., `{"message": "Validation Failed", "errors": [...]}`). The early exit drops this entirely — the LLM only sees `stderr` which might just be an unhelpful `gh: HTTP 422`.

This is both a **Risk 2 violation** (early exit before processing stdout) and a **Risk 1 issue** (hiding the actual failure reason from the LLM). With the current schema filter, even if stdout were processed, `{"message": "Not Found"}` becomes `{"message": "string"}` — the error reason is still hidden.

### Impact Analysis

| Dimension | Severity | Details |
|-----------|----------|---------|
| **Issue #188** | ~~Critical~~ **Fixed** | Full body now displayed |
| **gh api JSON** | High | Values replaced with type names — data destroyed |
| **gh api failure** | High | Error JSON in stdout dropped entirely on non-zero exit |
| **gh api non-JSON** | Medium | Hard truncation at 20 lines |
| **Workaround** | Low friction | `rtk proxy gh api` or raw `gh api` |

**Key context**: `gh api` is primarily used by automation scripts and LLM agents for programmatic access. The JSON schema filter was designed for `rtk json` (human exploration), not for API data pipelines. Applying it to `gh api` output is a design mismatch.

---

## 2. Risk Level Reassessment

The original HIGH rating was driven primarily by Issue #188 (full issue/PR bodies invisible). That is now fixed.

The remaining `gh api` issue (#199) is:
- **Lower impact**: `gh api` is less frequently used than `gh issue view` / `gh pr view`
- **Has workaround**: `rtk proxy gh api` bypasses all filtering
- **Contained**: Only affects the `run_api()` function (lines 1121-1163)

**Proposed downgrade**: HIGH → **LOW** after applying the fix below.

---

## 3. Proposed Solution

### Critical Safety Constraints

**Constraint 1: UTF-8 Safe String Truncation**

Rust strings are UTF-8 encoded. Slicing with `&s[..100]` operates on **byte indices**, not characters. If the 100th byte falls inside a multi-byte Unicode character (emoji, accented letter), this will `panic!` and crash RTK. GitHub issue/PR descriptions frequently contain emojis.

**Rule**: Never use `&s[..N]` for string truncation. Always use `s.chars().take(N).collect::<String>()`.

**Constraint 2: JSON String Escaping**

Using `format!("\"{}\"", s)` does not escape the inner string. If `s` contains double quotes or newlines (which markdown bodies always do), this produces malformed output that confuses LLMs.

**Rule**: Always use `serde_json::to_string(s)` for JSON string output. It handles all escaping automatically.

**Constraint 3: Stdout/Stderr Separation for JSON Parsing**

`filter_json_compact()` must be applied to `stdout` only. Concatenating `stderr` before JSON parsing will invalidate the JSON, causing fallback to raw line truncation even when stdout contains valid JSON.

**Rule**: Parse `stdout` as JSON independently. Print `stderr` separately. Combine only for `timer.track()`.

**Constraint 4: Risk 2 Invariant — `timer.track()` Before `process::exit()`**

`run_api()` currently has an early exit on failure that drops `stdout`. The fix must follow the standard RTK pattern:
```
1. println!(filtered)        // user sees output
2. timer.track(...)          // savings recorded to SQLite
3. std::process::exit(code)  // LAST — process terminates
```

### 3A. Replace JSON Schema Filter with Smart JSON Compression

Instead of converting values to type schemas (destroying data), apply a JSON compressor that **preserves values** while reducing token count:

1. **Truncate long string values** (>200 chars) — show first 100 chars + `...[N chars]`
2. **Collapse large arrays** — show first 3 elements + `... +N more`
3. **Limit object depth** — at max_depth, show `{...N keys}`
4. **Preserve all short values** — numbers, booleans, short strings, URLs

New function `filter_json_compact()` in `json_cmd.rs`:

```rust
/// Compact JSON for API output: preserves values, truncates long strings,
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
                // Safe char-based truncation (never panics on multi-byte chars)
                let truncated: String = s.chars().take(100).collect();
                let display = format!("{}...[{} chars]", truncated, s.chars().count());
                // Safe JSON escaping (handles quotes, newlines, control chars)
                serde_json::to_string(&display).unwrap_or_else(|_| "\"Error\"".to_string())
            } else {
                // Safe JSON escaping for all strings
                serde_json::to_string(s).unwrap_or_else(|_| "\"Error\"".to_string())
            }
        }
        Value::Array(arr) => {
            if arr.is_empty() {
                "[]".to_string()
            } else if arr.len() <= 3 {
                let items: Vec<String> = arr.iter()
                    .map(|v| compact_json(v, depth + 1, max_depth))
                    .collect();
                format!("[{}]", items.join(", "))
            } else {
                let items: Vec<String> = arr.iter().take(3)
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
        _ => value.to_string(), // numbers, bools, null: preserve as-is
    }
}
```

**Token savings**: Still significant (60%+) for large API responses — long markdown bodies truncated, large arrays collapsed — but all actual values preserved for programmatic use.

### 3B. Rewrite `run_api()` — Remove Early Exit, Process Stdout on Failure

The entire `run_api()` function must be restructured to follow the Risk 2 invariant. The current early exit on failure drops stdout (hiding API error details). The fix processes both stdout and stderr regardless of exit code:

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

**Key changes from current code:**
1. Removed early exit on failure — stdout is now processed even when `gh api` fails
2. Stdout and stderr processed separately — JSON parsing not broken by stderr
3. `timer.track()` called BEFORE `process::exit()` — tracking data preserved
4. Non-JSON truncation increased from 20 to 100 lines

### 3C. Increase Non-JSON Line Limit

The non-JSON fallback limit is increased from 20 to 100 lines (visible in 3B above).

**Rationale**: 20 lines is too aggressive for `gh api` which often returns GraphQL responses, error traces, or formatted text. 100 lines provides enough context while still capping extreme cases.

---

## 4. Files to Modify

| File | Change | Priority |
|------|--------|----------|
| `src/json_cmd.rs` | Add `filter_json_compact()` + `compact_json()` functions + tests | P0 |
| `src/gh_cmd.rs` | Rewrite `run_api()`: remove early exit, use `filter_json_compact()`, separate stdout/stderr, follow Risk 2 invariant | P0 |

---

## 5. What Does NOT Need Fixing

- **`view_issue()`** — Already shows full body via `filter_markdown_body()`. No changes needed.
- **`view_pr()`** — Already shows full body via `filter_markdown_body()`. No changes needed.
- **`filter_markdown_body()`** — Well-implemented, preserves code blocks, strips only noise. No changes needed.
- **`filter_json_string()`** — Keep as-is for `rtk json` command (schema inspection is the correct behavior there).

---

## 6. Risk Reduction Assessment

| Fix | Risk Reduction | Effort |
|-----|---------------|--------|
| `filter_json_compact()` for gh api | Preserves actual JSON values while saving tokens | Medium (1-2 hours) |
| Rewrite `run_api()` (remove early exit) | Error JSON no longer hidden; follows Risk 2 invariant | Small (30 min) |
| Increase non-JSON line limit | Reduces data loss for non-JSON responses | Small (15 min) |

**Expected risk level after implementation**: LOW — `gh api` preserves actual values, error details visible on failure, `view_issue` and `view_pr` already fixed, workaround available via `rtk proxy`.

---

## 7. Acceptance Criteria

- [ ] `rtk gh api repos/{owner}/{repo}/issues` returns actual issue data (titles, numbers, URLs) — not type schemas
- [ ] `rtk gh api` on HTTP 404/422 shows error JSON from stdout (not just stderr)
- [ ] `rtk gh api` still achieves 60%+ token savings on large API responses
- [ ] `rtk gh api` non-JSON output shows up to 100 lines before truncation
- [ ] `rtk json` behavior unchanged (still shows type schemas for exploration)
- [ ] String truncation handles Unicode safely (no panics on emoji/multi-byte chars)
- [ ] JSON string output properly escaped (no malformed quotes/newlines)
- [ ] `timer.track()` called before `process::exit()` in all paths
- [ ] `rtk gh issue view` still shows full body (regression check)
- [ ] `rtk gh pr view` still shows full body (regression check)
- [ ] All existing tests pass
- [ ] No performance regression

---

## 8. Resolved Questions

1. **Should `filter_json_compact()` be the default for all `gh` subcommands?** No — only change `run_api()`. Other `gh` subcommands already have hand-tuned JSON field extraction.

2. **Should the non-JSON limit be configurable?** No — keep it simple with a hardcoded 100. Users who need full output can use `rtk proxy gh api`.

3. **Should `filter_json_compact()` live in `json_cmd.rs` or `gh_cmd.rs`?** `json_cmd.rs` — it's a general-purpose JSON utility that other modules could reuse.

4. **How to safely truncate strings in Rust?** Use `s.chars().take(N).collect::<String>()` instead of `&s[..N]`. Byte slicing panics on multi-byte UTF-8 characters. GitHub content frequently contains emojis.

5. **How to safely format JSON strings?** Use `serde_json::to_string(s)` instead of `format!("\"{}\"", s)`. The latter doesn't escape inner quotes or newlines, producing malformed output.

6. **Should `run_api()` process stdout on failure?** Yes. GitHub API errors return JSON payloads in stdout (`{"message": "Not Found"}`). The early exit pattern drops this critical information. Fix: process stdout regardless of exit code, then exit at the end.
