# Implementation Plan: HIGH RISK 1 — Test Output Truncation Hides Failures

**Design Doc**: [01-high-risk-1-test-output-truncation.md](../design-docs/01-high-risk-1-test-output-truncation.md)
**Date**: 2026-02-22
**Issue**: [#221](https://github.com/rtk-ai/rtk/issues/221)

---

## Steps

### Step 1: Create test fixtures for prettier failure cases

Create `tests/fixtures/` directory and add two fixtures captured from real Prettier output.

**Files to create:**
- `tests/fixtures/prettier_check_failure.txt` — output from `prettier --check` when files need formatting
- `tests/fixtures/prettier_syntax_error.txt` — output from `prettier --check` when files have syntax errors

**Fixture content for `prettier_check_failure.txt`:**
```
Checking formatting...
[warn] src/components/Button.tsx
[warn] src/lib/utils.ts
[warn] src/pages/index.vue
[warn] config.yaml
Code style issues found in the above file(s). Forgot to run Prettier?
```

This covers: `.tsx`, `.ts`, `.vue`, `.yaml` — including extensions NOT in the current hardcoded list.

**Fixture content for `prettier_syntax_error.txt`:**
```
Checking formatting...
[error] src/broken.ts: SyntaxError: Unexpected token (3:1)
[error] src/bad.js: SyntaxError: Missing semicolon. (10:5)
[warn] src/messy.css
```

This covers: mixed errors + warnings in same run.

**Verification:** Files exist and are valid text.

---

### Step 2: Write failing tests first (TDD Red phase)

Add tests to `src/prettier_cmd.rs` `mod tests` that call `filter_prettier_output()` with the fixtures and assert correct behavior. These will FAIL against the current code — confirming the bug.

**Tests to add:**

```rust
#[test]
fn test_filter_warn_files_detected() {
    let output = include_str!("../tests/fixtures/prettier_check_failure.txt");
    let result = filter_prettier_output(output);
    // Must NOT claim success
    assert!(!result.contains("✓"), "Failure output must not contain success marker");
    assert!(!result.contains("All files formatted correctly"));
    // Must show file count
    assert!(result.contains("4 files need formatting"));
    // Must show all files including non-hardcoded extensions
    assert!(result.contains("Button.tsx"));
    assert!(result.contains("utils.ts"));
    assert!(result.contains("index.vue"));
    assert!(result.contains("config.yaml"));
}

#[test]
fn test_filter_error_lines_detected() {
    let output = include_str!("../tests/fixtures/prettier_syntax_error.txt");
    let result = filter_prettier_output(output);
    // Must NOT claim success
    assert!(!result.contains("✓"), "Error output must not contain success marker");
    assert!(!result.contains("All files formatted correctly"));
    // Must show errors
    assert!(result.contains("error") || result.contains("Error"));
    assert!(result.contains("broken.ts"));
    // Must also show the warn file
    assert!(result.contains("messy.css"));
}

#[test]
fn test_filter_empty_output_no_false_success() {
    // Empty or unrecognized output should NOT claim success
    let result = filter_prettier_output("");
    assert!(!result.contains("✓ Prettier: All files formatted correctly"));
}
```

**Run:** `cargo test prettier_cmd::tests` — expect 3 failures (confirming the bug exists).

---

### Step 3: Rewrite `filter_prettier_output()` in `src/prettier_cmd.rs`

Replace the current function (lines 47-139) with the fixed version.

**Key changes:**
1. Add `errors: Vec<String>` alongside `files_to_format`
2. Parse `[warn] <path>` lines via `strip_prefix("[warn] ")` → push to `files_to_format`
3. Parse `[error] <message>` lines via `strip_prefix("[error] ")` → push to `errors`
4. Remove the entire hardcoded extension check block (lines 61-77)
5. Fix success condition: only claim "all files formatted" if `files_to_format.is_empty() && errors.is_empty() && output.contains("All matched files use Prettier")`
6. Add error display section for syntax errors

**The rewritten function structure:**

```rust
pub fn filter_prettier_output(output: &str) -> String {
    let mut files_to_format: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut files_checked = 0;
    let mut is_check_mode = true;

    for line in output.lines() {
        let trimmed = line.trim();

        if let Some(path) = trimmed.strip_prefix("[warn] ") {
            files_to_format.push(path.to_string());
        } else if let Some(err) = trimmed.strip_prefix("[error] ") {
            errors.push(err.to_string());
        } else if trimmed.contains("Checking formatting") {
            is_check_mode = true;
        } else if trimmed.contains("All matched files use Prettier") {
            // Try to extract count from "All matched files use Prettier code style!"
            // (count is NOT in this line in modern Prettier, but keep for compat)
            if let Some(count_str) = trimmed.split_whitespace().next() {
                if let Ok(count) = count_str.parse::<usize>() {
                    files_checked = count;
                }
            }
        }
    }

    // Only claim success if explicitly confirmed AND no issues found
    if files_to_format.is_empty()
        && errors.is_empty()
        && output.contains("All matched files use Prettier")
    {
        return "✓ Prettier: All files formatted correctly".to_string();
    }

    // Check if files were written (write mode)
    if output.contains("modified") || output.contains("formatted") {
        is_check_mode = false;
    }

    let mut result = String::new();

    // Show errors first (syntax errors are more critical)
    if !errors.is_empty() {
        result.push_str(&format!("Prettier: {} errors\n", errors.len()));
        result.push_str("═══════════════════════════════════════\n");
        for err in errors.iter().take(10) {
            result.push_str(&format!("  {}\n", err));
        }
        if errors.len() > 10 {
            result.push_str(&format!("\n... +{} more errors\n", errors.len() - 10));
        }
        if !files_to_format.is_empty() {
            result.push('\n');
        }
    }

    if is_check_mode {
        if !files_to_format.is_empty() {
            result.push_str(&format!(
                "Prettier: {} files need formatting\n",
                files_to_format.len()
            ));
            if errors.is_empty() {
                result.push_str("═══════════════════════════════════════\n");
            }
            for (i, file) in files_to_format.iter().take(10).enumerate() {
                result.push_str(&format!("{}. {}\n", i + 1, file));
            }
            if files_to_format.len() > 10 {
                result.push_str(&format!(
                    "\n... +{} more files\n",
                    files_to_format.len() - 10
                ));
            }
            if files_checked > 0 && files_checked > files_to_format.len() {
                result.push_str(&format!(
                    "\n✓ {} files already formatted\n",
                    files_checked - files_to_format.len()
                ));
            }
        } else if errors.is_empty() {
            // No files_to_format AND no errors AND no "All matched" confirmation
            // Don't claim success — we don't know the state
            result.push_str("Prettier: no recognizable output\n");
        }
    } else {
        result.push_str(&format!(
            "✓ Prettier: {} files formatted\n",
            files_to_format.len()
        ));
    }

    result.trim().to_string()
}
```

**Run:** `cargo test prettier_cmd::tests` — expect all tests to pass (TDD Green phase).

---

### Step 4: Verify existing tests still pass

**Run:** `cargo test --all`

The three existing tests (`test_filter_all_formatted`, `test_filter_files_need_formatting`, `test_filter_many_files`) must still pass. They use inline fixtures without `[warn]` prefixes.

**Check:** `test_filter_files_need_formatting` uses bare file paths (`src/components/ui/button.tsx`) without `[warn]` prefix. In the new logic, these lines won't match `strip_prefix("[warn] ")` and won't be added to `files_to_format`. This test will **break**.

**Fix required:** Update `test_filter_files_need_formatting` fixture to use the `[warn]` prefix format that Prettier actually emits:
```rust
let output = r#"
Checking formatting...
[warn] src/components/ui/button.tsx
[warn] src/lib/auth/session.ts
[warn] src/pages/dashboard.tsx
Code style issues found in the above file(s). Forgot to run Prettier?
"#;
```

Similarly update `test_filter_many_files` to use `[warn]` prefix.

**Decision point:** Check if Prettier v2 (older) emits bare file paths without `[warn]`. If we need backward compat, add a fallback that detects bare file paths when no `[warn]` lines found. For now, target Prettier v3+ which always uses `[warn]`.

**Run:** `cargo test --all` — all green.

---

### Step 5: Add `ensure_failure_visibility()` to `src/utils.rs`

Add the systemic guard function at the end of `src/utils.rs` (before `mod tests`).

**Code to add:**

```rust
/// Safety net: if command failed but filtered output looks like success,
/// append a warning to the output string so LLMs see the mismatch.
///
/// This catches filter bugs where the filter incorrectly claims success
/// while the underlying command actually failed (non-zero exit code).
pub fn ensure_failure_visibility(filtered: &mut String, exit_code: i32, raw_stderr: &str) {
    if exit_code == 0 {
        return;
    }
    let dominated_by_success = filtered.starts_with("✓")
        || filtered.contains("No issues found")
        || filtered.to_lowercase().contains("all files formatted correctly")
        || filtered.contains("All tests passed");

    if dominated_by_success {
        filtered.push_str(&format!(
            "\n\n⚠️  Command exited with code {} but output looked clean.\n",
            exit_code
        ));
        if !raw_stderr.trim().is_empty() {
            let preview: String = raw_stderr.lines().take(5).collect::<Vec<_>>().join("\n");
            filtered.push_str(&format!("stderr: {}\n", preview));
        }
        filtered.push_str("Run raw command to verify.");
    }
}
```

**Test to add in `utils::tests`:**

```rust
#[test]
fn test_ensure_failure_visibility_mismatch() {
    let mut output = "✓ Prettier: All files formatted correctly".to_string();
    ensure_failure_visibility(&mut output, 1, "");
    assert!(output.contains("⚠️"));
    assert!(output.contains("exited with code 1"));
}

#[test]
fn test_ensure_failure_visibility_no_mismatch() {
    let mut output = "Prettier: 3 files need formatting".to_string();
    ensure_failure_visibility(&mut output, 1, "");
    assert!(!output.contains("⚠️"));
}

#[test]
fn test_ensure_failure_visibility_success_exit() {
    let mut output = "✓ Prettier: All files formatted correctly".to_string();
    ensure_failure_visibility(&mut output, 0, "");
    assert!(!output.contains("⚠️"));
}

#[test]
fn test_ensure_failure_visibility_with_stderr() {
    let mut output = "✓ ESLint: No issues found".to_string();
    ensure_failure_visibility(&mut output, 1, "Error: something went wrong\nline 2");
    assert!(output.contains("⚠️"));
    assert!(output.contains("something went wrong"));
}
```

**Run:** `cargo test utils::tests` — all green.

---

### Step 6: Integrate guard into `prettier_cmd.rs` `run()` function

In `src/prettier_cmd.rs`, call `ensure_failure_visibility()` after filtering, before printing.

**Change `run()` function** (current lines 23-29):

```rust
// Before (current):
let filtered = filter_prettier_output(&raw);
println!("{}", filtered);

// After:
let mut filtered = filter_prettier_output(&raw);
let exit_code = output.status.code().unwrap_or(if output.status.success() { 0 } else { 1 });
crate::utils::ensure_failure_visibility(&mut filtered, exit_code, &stderr);
println!("{}", filtered);
```

**Run:** `cargo test --all` — all green.

---

### Step 7: Integrate guard into 4 more high-risk filters

Apply the same pattern to vitest, cargo test, pytest, lint. For each, locate where `filtered` is printed and insert the guard call before it.

**Files and locations:**

1. **`src/vitest_cmd.rs`** — find where filtered output is printed, add guard before it
2. **`src/cargo_cmd.rs`** — find the test subcommand handler, add guard before printing
3. **`src/pytest_cmd.rs`** — find where filtered output is printed, add guard before it
4. **`src/lint_cmd.rs`** — at current line 179-182, add guard before `println!`

For each file:
- Change `let filtered = ...` to `let mut filtered = ...`
- Get `exit_code` from `output.status.code()`
- Call `crate::utils::ensure_failure_visibility(&mut filtered, exit_code, &stderr)`
- Run `cargo test` after each file

**Run:** `cargo test --all` — all green after each.

---

### Step 8: Run full quality gate

```bash
cargo fmt --all --check
cargo clippy --all-targets
cargo test --all
```

All three must pass with zero warnings.

---

### Step 9: Manual verification

Test the prettier fix manually (if a JS project with Prettier is available):

```bash
# Build
cargo build

# Test with intentionally unformatted file
target/debug/rtk prettier --check src/some-file.ts

# Verify: output shows file name, NOT "All files formatted correctly"
# Verify: exit code is non-zero
echo $?
```

If no Prettier project available, verify with unit tests only (already done in steps 2-4).

---

### Step 10: Commit

Single commit with all changes:
- `src/prettier_cmd.rs` — rewritten filter + new tests + guard integration
- `src/utils.rs` — `ensure_failure_visibility()` + tests
- `src/vitest_cmd.rs` — guard integration
- `src/cargo_cmd.rs` — guard integration
- `src/pytest_cmd.rs` — guard integration
- `src/lint_cmd.rs` — guard integration
- `tests/fixtures/prettier_check_failure.txt` — new fixture
- `tests/fixtures/prettier_syntax_error.txt` — new fixture

---

## Summary

| Step | What | Validates |
|------|------|-----------|
| 1 | Create test fixtures | Real Prettier output captured |
| 2 | Write failing tests (TDD Red) | Bug confirmed |
| 3 | Rewrite `filter_prettier_output()` (TDD Green) | #221 fixed, `[error]` fixed, extensions fixed |
| 4 | Verify existing tests | No regressions |
| 5 | Add `ensure_failure_visibility()` to utils | Systemic guard works |
| 6 | Integrate guard into prettier | Safety net for prettier |
| 7 | Integrate guard into 4 more filters | Safety net for vitest/cargo/pytest/lint |
| 8 | Full quality gate | fmt + clippy + test = clean |
| 9 | Manual verification | End-to-end works |
| 10 | Commit | All changes captured |
