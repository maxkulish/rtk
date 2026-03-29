# Upstream Sync v2 - Phase 1: Output Correctness

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix 8 output correctness bugs ported from upstream v0.30.0-v0.34.0 that cause wrong, missing, or truncated output.

**Architecture:** Each fix is isolated to 1-2 files. Tasks 1-3 modify `src/git.rs` in strict order (dependency chain). Tasks 4-7 are independent and can run in parallel.

**Tech Stack:** Rust, anyhow, regex, lazy_static, serde_json

**Spec:** `docs/superpowers/specs/2026-03-28-upstream-sync-v2-design.md` (Phase 1)

**Branch:** `sync/v2-p1-correctness`

**Pre-flight:** Run `cargo test 2>&1 | tail -1` to record baseline test count before starting.

---

## File Map

| File | Changes | Tasks |
|------|---------|-------|
| `src/git.rs` | Rewrite `filter_log_output`, update `compact_diff` truncation | 1, 2, 3 |
| `src/diff_cmd.rs` | Remove truncation from `condense_unified_diff` and `run()` | 4 |
| `src/main.rs` | Change read default from `"minimal"` to `"none"` | 5 |
| `src/read.rs` | Add binary file detection, add empty-filter fallback | 5 |
| `src/container.rs` | Fix double-unwrap, add exit code propagation | 6 |
| `src/log_cmd.rs` | Move 5 regexes to `lazy_static!` | 6 |
| `src/cargo_cmd.rs` | Add compile error detection fallback in test filter | 7 |

---

## Task 1: Preserve commit body in git log

**Upstream:** PR #546
**Files:**
- Modify: `src/git.rs:418` (format string)
- Modify: `src/git.rs:478-494` (rewrite `filter_log_output`)

- [ ] **Step 1: Write failing test for body extraction**

Add to the `#[cfg(test)] mod tests` block in `src/git.rs`:

```rust
#[test]
fn test_filter_log_output_preserves_body() {
    let input = "abc1234 feat: add feature (2 days ago) <dev>\n\
                 This commit adds a new feature for users.\n\
                 \n\
                 Signed-off-by: Dev <dev@example.com>\n\
                 ---END---\n\
                 def5678 fix: bug fix (3 days ago) <dev>\n\
                 \n\
                 ---END---";
    let result = filter_log_output(input, 10, false, false);
    assert!(result.contains("abc1234 feat: add feature"));
    assert!(result.contains("  This commit adds a new feature"));
    assert!(!result.contains("Signed-off-by"));
    assert!(result.contains("def5678 fix: bug fix"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test test_filter_log_output_preserves_body -- --nocapture`
Expected: FAIL - `filter_log_output` takes 2 args, not 4

- [ ] **Step 3: Update format string in run_log**

In `src/git.rs`, change line 418:

```rust
// Old:
cmd.args(["--pretty=format:%h %s (%ar) <%an>"]);
// New:
cmd.args(["--pretty=format:%h %s (%ar) <%an>%n%b%n---END---"]);
```

- [ ] **Step 4: Rewrite filter_log_output to parse blocks**

Replace `filter_log_output` (lines 478-494) with:

```rust
/// Filter git log output: parse blocks separated by ---END---, extract body
fn filter_log_output(output: &str, limit: usize, user_set_limit: bool, user_format: bool) -> String {
    let truncate_width = 120;

    // When user specified their own format, don't parse ---END--- blocks
    if user_format {
        let lines: Vec<&str> = output.lines().collect();
        let max_lines = if user_set_limit { lines.len() } else { limit };
        return lines
            .iter()
            .take(max_lines)
            .map(|l| {
                if l.len() > truncate_width {
                    let truncated: String = l.chars().take(truncate_width - 3).collect();
                    format!("{}...", truncated)
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
    }

    // Parse ---END--- delimited blocks from RTK's custom format
    let blocks: Vec<&str> = output.split("---END---").collect();
    let mut entries = Vec::new();

    for block in blocks.iter().take(limit) {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }

        let mut lines = block.lines();
        let header = match lines.next() {
            Some(h) => h.trim(),
            None => continue,
        };

        if header.is_empty() {
            continue;
        }

        // Truncate header if too long
        let header = if header.len() > truncate_width {
            let truncated: String = header.chars().take(truncate_width - 3).collect();
            format!("{}...", truncated)
        } else {
            header.to_string()
        };

        let mut entry = header;

        // Extract first meaningful body line (skip trailers)
        let all_body_lines: Vec<&str> = lines
            .map(|l| l.trim())
            .filter(|l| {
                !l.is_empty()
                    && !l.starts_with("Signed-off-by:")
                    && !l.starts_with("Co-authored-by:")
                    && !l.starts_with("Co-Authored-By:")
            })
            .collect();

        // Show up to 3 body lines
        let body_limit = 3;
        let body_omitted = all_body_lines.len().saturating_sub(body_limit);
        let body_lines = &all_body_lines[..all_body_lines.len().min(body_limit)];

        for body_line in body_lines {
            let body_line = if body_line.len() > truncate_width - 2 {
                let truncated: String = body_line.chars().take(truncate_width - 5).collect();
                format!("{}...", truncated)
            } else {
                body_line.to_string()
            };
            entry.push_str(&format!("\n  {}", body_line));
        }

        if body_omitted > 0 {
            entry.push_str(&format!("\n  [+{} lines omitted]", body_omitted));
        }

        entries.push(entry);
    }

    entries.join("\n")
}
```

- [ ] **Step 5: Update call site in run_log**

In `run_log()`, change the call at line 464:

```rust
// Old:
let filtered = filter_log_output(&stdout, limit);
// New:
let filtered = filter_log_output(&stdout, limit, has_limit_flag, has_format_flag);
```

- [ ] **Step 6: Update existing tests**

Find all existing calls to `filter_log_output` in tests and update them to pass 4 arguments. For tests that use the old line-based format, update input to use `---END---` delimited format. For tests that test with user formats like `--oneline`, pass `true` for `user_format`.

- [ ] **Step 7: Run all tests**

Run: `cargo test git::tests -- --nocapture`
Expected: All git tests pass

- [ ] **Step 8: Run full quality check**

Run: `cargo fmt --all && cargo clippy --all-targets && cargo test`
Expected: No warnings, all tests pass

- [ ] **Step 9: Commit**

```bash
git add src/git.rs
git commit -m "fix(sync): preserve commit body in git log output (upstream #546)"
```

---

## Task 2: Fix git log --oneline regression

**Upstream:** PR #619
**Depends on:** Task 1
**Files:**
- Modify: `src/git.rs` (`filter_log_output` - already updated in Task 1)

The `user_format` branch was already added in Task 1's `filter_log_output` rewrite. This task adds tests to verify the behavior and ensures the implementation is correct.

- [ ] **Step 1: Write test for --oneline preservation**

Add to `src/git.rs` tests:

```rust
#[test]
fn test_filter_log_output_user_format_oneline() {
    // Simulates --oneline output (no ---END--- markers)
    let input = "abc1234 first commit\n\
                 def5678 second commit\n\
                 ghi9012 third commit\n\
                 jkl3456 fourth commit\n\
                 mno7890 fifth commit";
    // user_format=true, user_set_limit=false, limit=3 (RTK default)
    let result = filter_log_output(input, 3, false, true);
    let lines: Vec<&str> = result.lines().collect();
    assert_eq!(lines.len(), 3, "Should show 3 lines (RTK default limit)");
    assert!(result.contains("abc1234 first commit"));
    assert!(result.contains("ghi9012 third commit"));
    assert!(!result.contains("jkl3456"), "4th commit should be truncated");
}

#[test]
fn test_filter_log_output_user_format_respects_user_limit() {
    let input = "abc1234 first commit\n\
                 def5678 second commit\n\
                 ghi9012 third commit";
    // user_format=true, user_set_limit=true -> show ALL lines
    let result = filter_log_output(input, 3, true, true);
    let lines: Vec<&str> = result.lines().collect();
    assert_eq!(lines.len(), 3, "Should show all 3 lines when user set limit");
}

#[test]
fn test_filter_log_output_user_format_no_drops() {
    // Regression test: --oneline must NOT drop commits
    let input = (0..20)
        .map(|i| format!("{:07x} commit {}", i, i))
        .collect::<Vec<_>>()
        .join("\n");
    let result = filter_log_output(&input, 50, false, true);
    let lines: Vec<&str> = result.lines().collect();
    assert_eq!(lines.len(), 20, "All 20 commits must be present");
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test test_filter_log_output_user_format -- --nocapture`
Expected: All 3 tests pass (implementation was done in Task 1)

- [ ] **Step 3: Run full quality check**

Run: `cargo fmt --all && cargo clippy --all-targets && cargo test`
Expected: All pass

- [ ] **Step 4: Commit**

```bash
git add src/git.rs
git commit -m "fix(sync): prevent git log --oneline from dropping commits (upstream #619)"
```

---

## Task 3: Exact truncation counts in compact_diff

**Upstream:** PR #833
**Depends on:** Task 2
**Files:**
- Modify: `src/git.rs:303-367` (`compact_diff`)

- [ ] **Step 1: Write test for exact hunk truncation count**

Add to `src/git.rs` tests:

```rust
#[test]
fn test_compact_diff_hunk_truncation_count_accurate() {
    // Build a diff with 60 added lines in one hunk
    let mut diff = String::from("diff --git a/big.rs b/big.rs\n");
    diff.push_str("--- a/big.rs\n+++ b/big.rs\n");
    diff.push_str("@@ -1,0 +1,60 @@\n");
    for i in 0..60 {
        diff.push_str(&format!("+line {}\n", i));
    }

    let result = compact_diff(&diff, 500);
    // Should show first 10 lines, then exact count of remaining
    assert!(
        result.contains("50 lines truncated"),
        "Should show exact count of truncated lines, got: {}",
        result
    );
}

#[test]
fn test_compact_diff_no_false_truncation() {
    // Diff with exactly 8 added lines - no truncation needed
    let mut diff = String::from("diff --git a/small.rs b/small.rs\n");
    diff.push_str("--- a/small.rs\n+++ b/small.rs\n");
    diff.push_str("@@ -1,0 +1,8 @@\n");
    for i in 0..8 {
        diff.push_str(&format!("+line {}\n", i));
    }

    let result = compact_diff(&diff, 500);
    assert!(
        !result.contains("truncated"),
        "8 lines should not trigger truncation, got: {}",
        result
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test test_compact_diff_hunk_truncation -- --nocapture`
Expected: FAIL - current code shows `"... (truncated)"` not `"50 lines truncated"`

- [ ] **Step 3: Rewrite compact_diff with exact counts**

Replace `compact_diff` (lines 303-367) with:

```rust
pub(crate) fn compact_diff(diff: &str, max_lines: usize) -> String {
    let mut result = Vec::new();
    let mut current_file = String::new();
    let mut added = 0;
    let mut removed = 0;
    let mut in_hunk = false;
    let mut hunk_shown = 0;
    let mut hunk_skipped: usize = 0;
    let max_hunk_lines = 10;

    for line in diff.lines() {
        if line.starts_with("diff --git") {
            // Flush previous hunk's skipped count
            if hunk_skipped > 0 {
                result.push(format!("  ... ({} lines truncated)", hunk_skipped));
                hunk_skipped = 0;
            }
            // New file
            if !current_file.is_empty() && (added > 0 || removed > 0) {
                result.push(format!("  +{} -{}", added, removed));
            }
            current_file = line.split(" b/").nth(1).unwrap_or("unknown").to_string();
            result.push(format!("\n📄 {}", current_file));
            added = 0;
            removed = 0;
            in_hunk = false;
        } else if line.starts_with("@@") {
            // Flush previous hunk's skipped count
            if hunk_skipped > 0 {
                result.push(format!("  ... ({} lines truncated)", hunk_skipped));
                hunk_skipped = 0;
            }
            // New hunk
            in_hunk = true;
            hunk_shown = 0;
            let hunk_info = line.split("@@").nth(1).unwrap_or("").trim();
            result.push(format!("  @@ {} @@", hunk_info));
        } else if in_hunk {
            if line.starts_with('+') && !line.starts_with("+++") {
                added += 1;
                if hunk_shown < max_hunk_lines {
                    result.push(format!("  {}", line));
                    hunk_shown += 1;
                } else {
                    hunk_skipped += 1;
                }
            } else if line.starts_with('-') && !line.starts_with("---") {
                removed += 1;
                if hunk_shown < max_hunk_lines {
                    result.push(format!("  {}", line));
                    hunk_shown += 1;
                } else {
                    hunk_skipped += 1;
                }
            } else if !line.starts_with("\\") {
                // Context line
                if hunk_shown < max_hunk_lines && hunk_shown > 0 {
                    result.push(format!("  {}", line));
                    hunk_shown += 1;
                } else if hunk_shown >= max_hunk_lines {
                    hunk_skipped += 1;
                }
            }
        }

        if result.len() >= max_lines {
            if hunk_skipped > 0 {
                result.push(format!("  ... ({} lines truncated)", hunk_skipped));
            }
            result.push("\n[full diff: rtk git diff --no-compact]".to_string());
            break;
        }
    }

    // Flush final hunk's skipped count
    if hunk_skipped > 0 {
        result.push(format!("  ... ({} lines truncated)", hunk_skipped));
    }

    if !current_file.is_empty() && (added > 0 || removed > 0) {
        result.push(format!("  +{} -{}", added, removed));
    }

    result.join("\n")
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test compact_diff -- --nocapture`
Expected: All compact_diff tests pass, including new ones

- [ ] **Step 5: Write test for body omission indicator**

Add to tests:

```rust
#[test]
fn test_filter_log_output_body_omission_indicator() {
    let body_lines = (0..6)
        .map(|i| format!("Body line {}", i))
        .collect::<Vec<_>>()
        .join("\n");
    let input = format!("abc1234 feat: big change (1 day ago) <dev>\n{}\n---END---", body_lines);
    let result = filter_log_output(&input, 10, false, false);
    assert!(result.contains("Body line 0"), "First body line should show");
    assert!(result.contains("Body line 2"), "Third body line should show");
    assert!(result.contains("[+3 lines omitted]"), "Should indicate 3 omitted lines, got: {}", result);
}
```

- [ ] **Step 6: Run tests and verify**

Run: `cargo test test_filter_log_output_body -- --nocapture`
Expected: Pass (body handling was implemented in Task 1)

- [ ] **Step 7: Run full quality check**

Run: `cargo fmt --all && cargo clippy --all-targets && cargo test`
Expected: All pass

- [ ] **Step 8: Commit**

```bash
git add src/git.rs
git commit -m "fix(sync): exact truncation counts in compact_diff (upstream #833)"
```

---

## Task 4: Never truncate diff content

**Upstream:** PR #827
**Independent of:** Tasks 1-3
**Files:**
- Modify: `src/diff_cmd.rs:159-212` (`condense_unified_diff`)

- [ ] **Step 1: Write test for large diff preservation**

Add to `src/diff_cmd.rs` tests:

```rust
#[test]
fn test_condense_unified_diff_no_truncation() {
    // Build a diff with 200 added lines
    let mut diff = String::from("--- a/big.rs\n+++ b/big.rs\n");
    diff.push_str("@@ -1,0 +1,200 @@\n");
    for i in 0..200 {
        diff.push_str(&format!("+line number {}\n", i));
    }

    let result = condense_unified_diff(&diff);
    // All 200 lines should be present - no truncation
    for i in 0..200 {
        assert!(
            result.contains(&format!("+line number {}", i)),
            "Line {} should be present in output",
            i
        );
    }
    assert!(
        !result.contains("more"),
        "Should have no truncation indicator, got: {}",
        result
    );
}

#[test]
fn test_condense_unified_diff_long_lines_preserved() {
    let long_line = format!("+{}", "x".repeat(500));
    let diff = format!("--- a/file.rs\n+++ b/file.rs\n@@ -1 +1 @@\n{}\n", long_line);
    let result = condense_unified_diff(&diff);
    assert!(
        result.contains(&"x".repeat(500)),
        "Long lines should not be truncated"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test test_condense_unified_diff_no_truncation -- --nocapture`
Expected: FAIL - current code caps at 15 changes and `.take(10)` display

- [ ] **Step 3: Remove truncation from condense_unified_diff**

Replace `condense_unified_diff` (lines 159-212) with:

```rust
fn condense_unified_diff(diff: &str) -> String {
    let mut result = Vec::new();
    let mut current_file = String::new();
    let mut added = 0;
    let mut removed = 0;
    let mut changes = Vec::new();

    for line in diff.lines() {
        if line.starts_with("diff --git") || line.starts_with("--- ") || line.starts_with("+++ ") {
            if line.starts_with("+++ ") {
                if !current_file.is_empty() && (added > 0 || removed > 0) {
                    result.push(format!("📄 {} (+{} -{})", current_file, added, removed));
                    for c in &changes {
                        result.push(format!("  {}", c));
                    }
                }
                current_file = line
                    .trim_start_matches("+++ ")
                    .trim_start_matches("b/")
                    .to_string();
                added = 0;
                removed = 0;
                changes.clear();
            }
        } else if line.starts_with('+') && !line.starts_with("+++") {
            added += 1;
            changes.push(line.to_string());
        } else if line.starts_with('-') && !line.starts_with("---") {
            removed += 1;
            changes.push(line.to_string());
        }
    }

    // Last file
    if !current_file.is_empty() && (added > 0 || removed > 0) {
        result.push(format!("📄 {} (+{} -{})", current_file, added, removed));
        for c in &changes {
            result.push(format!("  {}", c));
        }
    }

    result.join("\n")
}
```

- [ ] **Step 4: Remove truncate import if unused**

Check if `truncate` from `crate::utils` is still used elsewhere in `diff_cmd.rs`. If not, remove the import.

- [ ] **Step 5: Run tests**

Run: `cargo test diff_cmd -- --nocapture`
Expected: All diff_cmd tests pass

- [ ] **Step 6: Run full quality check**

Run: `cargo fmt --all && cargo clippy --all-targets && cargo test`
Expected: All pass

- [ ] **Step 7: Commit**

```bash
git add src/diff_cmd.rs
git commit -m "fix(sync): never truncate diff content (upstream #827)"
```

---

## Task 5: Read defaults to no filtering + binary file detection

**Upstream:** PR #824 (commits `5e0f3ba`, `8886c14`)
**Independent of:** Tasks 1-4
**Files:**
- Modify: `src/main.rs:108` (default_value)
- Modify: `src/read.rs` (binary detection, empty fallback)

- [ ] **Step 1: Write test for binary file detection**

Add to `src/read.rs` tests (create `#[cfg(test)] mod tests` if not present):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_binary_detects_null_bytes() {
        let data = b"hello\x00world";
        assert!(is_likely_binary(data));
    }

    #[test]
    fn test_is_binary_passes_text() {
        let data = b"fn main() {\n    println!(\"hello\");\n}";
        assert!(!is_likely_binary(data));
    }

    #[test]
    fn test_is_binary_passes_utf8() {
        let data = "日本語のコード".as_bytes();
        assert!(!is_likely_binary(data));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test read::tests -- --nocapture`
Expected: FAIL - `is_likely_binary` function not found

- [ ] **Step 3: Add binary detection function to read.rs**

Add at the top of `src/read.rs`:

```rust
/// Check if data is likely binary by scanning for null bytes in first 8KB
fn is_likely_binary(data: &[u8]) -> bool {
    let check_len = data.len().min(8192);
    data[..check_len].contains(&0)
}

/// Format bytes into human-readable size
fn human_size(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} bytes", bytes)
    }
}
```

- [ ] **Step 4: Update run() to use binary detection and empty-filter fallback**

In `src/read.rs`, modify the `run()` function to:
1. Read file as bytes first (`fs::read`)
2. Check for binary content
3. Convert to string
4. After filtering, check if result is empty and fallback to raw content

```rust
pub fn run(
    file: &Path,
    level: FilterLevel,
    max_lines: Option<usize>,
    line_numbers: bool,
    verbose: u8,
) -> Result<()> {
    let raw_bytes = std::fs::read(file)
        .with_context(|| format!("Failed to read file: {}", file.display()))?;

    // Binary file detection
    if is_likely_binary(&raw_bytes) {
        let size = human_size(raw_bytes.len() as u64);
        println!("[binary file: {} ({})]", file.display(), size);
        println!("hint: use cat {} to view raw content", file.display());
        return Ok(());
    }

    let content = String::from_utf8(raw_bytes)
        .with_context(|| format!("File is not valid UTF-8: {}", file.display()))?;

    // ... rest of existing function using `content` instead of reading from file ...
    // Apply filter
    let filter = filter::get_filter(level);
    let mut filtered = filter.filter(&content);

    // Safety: if filter produced empty output from non-empty input, warn and fallback
    if filtered.trim().is_empty() && !content.trim().is_empty() {
        eprintln!(
            "rtk: warning: filter produced empty output for {} ({} bytes), showing raw content",
            file.display(),
            content.len()
        );
        filtered = content.clone();
    }

    // ... continue with existing max_lines/line_numbers logic using `filtered` ...
}
```

The key changes vs the existing `run()` function are: (1) read with `fs::read` instead of `fs::read_to_string`, (2) add the `is_likely_binary` check before UTF-8 conversion, (3) after filtering, add the empty-output fallback. Keep all existing `max_lines` truncation and `line_numbers` formatting logic unchanged.

- [ ] **Step 5: Change default filter level in main.rs**

In `src/main.rs`, line 108:

```rust
// Old:
#[arg(short, long, default_value = "minimal")]
// New:
#[arg(short, long, default_value = "none")]
```

- [ ] **Step 6: Run tests**

Run: `cargo test read -- --nocapture`
Expected: All pass

- [ ] **Step 7: Run full quality check**

Run: `cargo fmt --all && cargo clippy --all-targets && cargo test`
Expected: All pass

- [ ] **Step 8: Commit**

```bash
git add src/read.rs src/main.rs
git commit -m "fix(sync): read defaults to no filtering, detect binary files (upstream #822)"
```

---

## Task 6: Critical bugs - container double-unwrap + log_cmd lazy regex

**Upstream:** PR #626
**Independent of:** Tasks 1-5
**Files:**
- Modify: `src/container.rs:27-87` (docker_ps exit code)
- Modify: `src/container.rs:89-152` (docker_images exit code)
- Modify: `src/container.rs:212-220` (kubectl_pods double-unwrap)
- Modify: `src/container.rs:313-321` (kubectl_services double-unwrap)
- Modify: `src/log_cmd.rs:62-69` (regex -> lazy_static)

- [ ] **Step 1: Write test for idiomatic Option handling**

Add to `src/container.rs` tests:

```rust
#[test]
fn test_kubectl_pods_empty_items() {
    // Verify that empty items array is handled without panics
    let json: serde_json::Value = serde_json::json!({
        "items": []
    });
    let items = json["items"].as_array().filter(|a| !a.is_empty());
    assert!(items.is_none(), "Empty items should become None");
}

#[test]
fn test_kubectl_pods_missing_items() {
    let json: serde_json::Value = serde_json::json!({});
    let items = json["items"].as_array().filter(|a| !a.is_empty());
    assert!(items.is_none(), "Missing items should become None");
}
```

- [ ] **Step 2: Run tests to verify they pass (these test the new pattern)**

Run: `cargo test test_kubectl_pods -- --nocapture`
Expected: PASS (testing the target pattern)

- [ ] **Step 3: Fix kubectl_pods double-unwrap**

In `src/container.rs`, replace lines 212-220:

```rust
// Old:
let items = json["items"].as_array();
if items.is_none() || items.unwrap().is_empty() {
    rtk.push_str("☸️  No pods found");
    println!("{}", rtk);
    timer.track("kubectl get pods", "rtk kubectl pods", &raw, &rtk);
    return Ok(());
}

let pods = items.unwrap();

// New:
let Some(pods) = json["items"].as_array().filter(|a| !a.is_empty()) else {
    rtk.push_str("☸️  No pods found");
    println!("{}", rtk);
    timer.track("kubectl get pods", "rtk kubectl pods", &raw, &rtk);
    return Ok(());
};
```

- [ ] **Step 4: Fix kubectl_services double-unwrap**

In `src/container.rs`, replace lines 313-321 with the same pattern:

```rust
// Old:
let items = json["items"].as_array();
if items.is_none() || items.unwrap().is_empty() {
    ...
}
let services = items.unwrap();

// New:
let Some(services) = json["items"].as_array().filter(|a| !a.is_empty()) else {
    rtk.push_str("☸️  No services found");
    println!("{}", rtk);
    timer.track("kubectl get svc", "rtk kubectl svc", &raw, &rtk);
    return Ok(());
};
```

- [ ] **Step 5: Add exit code propagation to docker_ps**

In `docker_ps()`, after line 43 (`context("Failed to run docker ps")?`), add:

```rust
if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    eprint!("{}", stderr);
    timer.track("docker ps", "rtk docker ps", &raw, &raw);
    std::process::exit(output.status.code().unwrap_or(1));
}
```

Note: The first `Command::new("docker")` call on line 30-34 uses `.unwrap_or_default()` for raw capture - that's fine. The exit code check goes on the second call (line 36-43) which is the formatted output.

- [ ] **Step 6: Add exit code propagation to docker_images**

Same pattern in `docker_images()`, after line 101:

```rust
if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    eprint!("{}", stderr);
    timer.track("docker images", "rtk docker images", &raw, &raw);
    std::process::exit(output.status.code().unwrap_or(1));
}
```

- [ ] **Step 7: Move log_cmd regexes to lazy_static**

In `src/log_cmd.rs`, add at the top of the file (after imports):

```rust
use lazy_static::lazy_static;

lazy_static! {
    static ref TIMESTAMP_RE: Regex =
        Regex::new(r"^\d{4}[-/]\d{2}[-/]\d{2}[T ]\d{2}:\d{2}:\d{2}[.,]?\d*\s*").unwrap();
    static ref UUID_RE: Regex =
        Regex::new(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}").unwrap();
    static ref HEX_RE: Regex = Regex::new(r"0x[0-9a-fA-F]+").unwrap();
    static ref NUM_RE: Regex = Regex::new(r"\b\d{4,}\b").unwrap();
    static ref PATH_RE: Regex = Regex::new(r"/[\w./\-]+").unwrap();
}
```

Then remove the 5 `let ... = Regex::new(...)` lines from inside `analyze_logs()` (lines 62-69) and update all references from `timestamp_re` to `&TIMESTAMP_RE`, etc. The regex variables are used in `normalize_log_line()` calls - update those call sites to use the statics.

- [ ] **Step 8: Run full quality check**

Run: `cargo fmt --all && cargo clippy --all-targets && cargo test`
Expected: All pass

- [ ] **Step 9: Commit**

```bash
git add src/container.rs src/log_cmd.rs
git commit -m "fix(sync): container exit codes, idiomatic Option, lazy regex (upstream #626)"
```

---

## Task 7: Preserve cargo test compile diagnostics

**Upstream:** PR #738
**Independent of:** Tasks 1-6
**Files:**
- Modify: `src/cargo_cmd.rs` (`filter_cargo_test`)

- [ ] **Step 1: Write failing test for compile error preservation**

Add to `src/cargo_cmd.rs` tests:

```rust
#[test]
fn test_filter_cargo_test_compile_error_preserves_diagnostics() {
    let input = r#"   Compiling myapp v0.1.0 (/home/user/myapp)
error[E0308]: mismatched types
 --> src/main.rs:10:5
  |
10|     let x: u32 = "hello";
  |                  ^^^^^^^ expected `u32`, found `&str`

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0308`.
error: could not compile `myapp` (bin "myapp") due to 1 previous error
"#;
    let result = filter_cargo_test(input);
    assert!(
        result.contains("error[E0308]") || result.contains("mismatched types"),
        "Compile errors should be preserved, got: {}",
        result
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test test_filter_cargo_test_compile_error -- --nocapture`
Expected: FAIL - current filter strips everything (no `test result:` line, no failures section -> empty result)

- [ ] **Step 3: Add compile error detection fallback**

In `filter_cargo_test()`, after line 762 (`failures.push(current_failure.join("\n"));`), before the `let mut result = String::new();` line, add the compile error detection:

```rust
    // If no test results found, check for compile errors
    if failures.is_empty() && summary_lines.is_empty() {
        let has_compile_errors = output.lines().any(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with("error[") || trimmed.starts_with("error:")
        });
        if has_compile_errors {
            let build_filtered = filter_cargo_build(output);
            if build_filtered.starts_with("cargo build:") {
                return build_filtered.replacen("cargo build:", "cargo test:", 1);
            }
            // If filter_cargo_build didn't produce structured output, return it as-is
            if !build_filtered.is_empty() {
                return build_filtered;
            }
        }
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test filter_cargo_test -- --nocapture`
Expected: All pass including new test

- [ ] **Step 5: Run full quality check**

Run: `cargo fmt --all && cargo clippy --all-targets && cargo test`
Expected: All pass

- [ ] **Step 6: Commit**

```bash
git add src/cargo_cmd.rs
git commit -m "fix(sync): preserve cargo test compile diagnostics (upstream #738)"
```

---

## Final Quality Gate

After all 7 tasks:

- [ ] **Run full test suite**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test
```

- [ ] **Build release binary and check size**

```bash
cargo build --release
ls -lh target/release/rtk
# Must be < 5MB
```

- [ ] **Manual verification tests**

```bash
# Git log body preservation
rtk git log -3
# Should show commit body lines indented under headers

# Git log --oneline
rtk git log --oneline -5
# Should show exactly 5 commits, none dropped

# Git log -n N
rtk git log -n 3
# Should show exactly 3 commits

# Diff - no truncation
rtk git diff HEAD~1
# Should show full diff content

# Read - no filtering by default
rtk read Cargo.toml
# Should show full file including comments

# Cargo test with compile error (if available)
# rtk cargo test -- broken_test
```

- [ ] **Merge to master**

```bash
git checkout master
git merge sync/v2-p1-correctness
git push origin master
```
