# Design Document: HIGH RISK 1 — Test Output Truncation Hides Failures

**Date**: 2026-02-22
**Risk Level**: HIGH
**Status**: Revised (v2) — incorporating review feedback
**Related Issues**: [#221](https://github.com/rtk-ai/rtk/issues/221) (rtk prettier --check)

---

## 1. Problem Statement

RTK filters are designed to reduce token consumption by removing verbose output. However, when a filter has a bug, it can misclassify a **failure as a pass**, producing a false-negative. The LLM (or developer) sees "all good" when the underlying tool actually reported problems.

This is the most dangerous class of RTK bug because it's **silent** — there's no error, no crash, just incorrect information that leads to wrong decisions.

### Current Manifestation: Issue #221

`rtk prettier --check` incorrectly reports "All files formatted correctly" when Prettier exits with code 1 (files need formatting).

**Root cause** in `src/prettier_cmd.rs:61-66`:
```rust
if !trimmed.is_empty()
    && !trimmed.starts_with("Checking")
    && !trimmed.starts_with("All matched")
    && !trimmed.starts_with("Code style")
    && !trimmed.contains("[warn]")
    && !trimmed.contains("[error]")
    && (trimmed.ends_with(".ts") || ... hardcoded extensions ...)
```

**Two interacting bugs:**

1. **`[warn]` stripping**: Prettier's `--check` mode reports unformatted files using the `[warn]` prefix (`[warn] src/messy.ts`). The filter explicitly excludes these lines, so `files_to_format` stays empty.

2. **Empty `files_to_format` always claims success**: At line 103, if `files_to_format.is_empty()`, it unconditionally prints `"✓ Prettier: All files formatted correctly"` — regardless of exit code or whether "All matched files use Prettier" appeared in output.

Combined, these mean Prettier can exit 1 (failure) while RTK prints "all good".

**Contradiction**: The exit code IS preserved correctly (line 39-41 calls `process::exit`), but the printed output says the opposite. When an LLM reads RTK output, it trusts the text — not the exit code.

### Blind Spot: `[error]` Tag (Syntax Errors)

Prettier uses the `[error]` prefix for syntax errors it cannot parse:
```
Checking formatting...
[error] src/bad.ts: SyntaxError: Unexpected token (1:1)
```

The same `!trimmed.contains("[error]")` condition strips these lines. RTK tells the LLM "all files formatted correctly" when Prettier actually crashed on syntax errors. This must be fixed alongside `[warn]`.

### Blind Spot: Hardcoded File Extensions

The current filter manually checks 8 extensions (`.ts`, `.tsx`, `.js`, `.jsx`, `.json`, `.md`, `.css`, `.scss`). Prettier also supports `.html`, `.yaml`, `.yml`, `.graphql`, `.vue`, `.svelte`, `.less`, `.xml`, and more.

Any project using Prettier for unsupported extensions will have those files silently ignored — `files_to_format` stays empty, false success reported.

**Resolution**: Parse `[warn] <filepath>` lines directly instead of relying on extension matching. The `[warn]` prefix IS the signal.

### Broader Pattern

This bug class can exist in ANY RTK filter:
1. Filter strips lines that look like noise/metadata
2. Those lines actually contain critical failure information
3. Output says "success" while the exit code says "failure"
4. LLM reads text output, ignores exit code → wrong conclusion

Other potentially vulnerable filters:
- `vitest_cmd.rs` — strips ANSI, could miss failure markers
- `cargo_cmd.rs` — shows "failures only", could misparse failure detection
- `pytest_cmd.rs` — state machine parser could miss failure transitions
- `lint_cmd.rs` — generic linter fallback uses keyword matching

---

## 2. Impact Analysis

| Dimension | Severity | Details |
|-----------|----------|---------|
| **Correctness** | Critical | LLM makes decisions based on false information |
| **Trust** | Critical | Users can't trust RTK output for quality gates |
| **CI/CD** | High | Output says pass, code says fail — confusing |
| **Discoverability** | Low | Silent failure — user may never notice |

**Worst case**: LLM sees "all tests pass" / "all files formatted" → commits and pushes code with real failures → CI catches it (if not also using RTK) or bugs reach production.

---

## 3. Proposed Solution: Output-Exit Code Consistency Enforcement

### 3A. Immediate Fix: Prettier Filter (Issue #221)

Rewrite `filter_prettier_output()` to parse `[warn]` and `[error]` prefixed lines instead of stripping them.

**Changes in `src/prettier_cmd.rs`**:

1. **Parse `[warn] <filepath>` lines** — strip the `[warn] ` prefix, add remaining path to `files_to_format`
2. **Parse `[error] <message>` lines** — collect into a separate `errors` vec for syntax errors
3. **Remove hardcoded extension list** — the `[warn]` prefix is the reliable signal, not file extension
4. **Fix empty `files_to_format` logic** — only claim success if BOTH `files_to_format` is empty AND `errors` is empty AND the output explicitly contains "All matched files use Prettier"

Revised filter logic:
```rust
for line in output.lines() {
    let trimmed = line.trim();

    if let Some(path) = trimmed.strip_prefix("[warn] ") {
        files_to_format.push(path.to_string());
    } else if let Some(err) = trimmed.strip_prefix("[error] ") {
        errors.push(err.to_string());
    } else if trimmed.contains("Checking formatting") {
        is_check_mode = true;
    }
    // ... handle "All matched files" for files_checked count
}

// Only claim success if nothing went wrong
if files_to_format.is_empty() && errors.is_empty()
    && output.contains("All matched files use Prettier")
{
    return "✓ Prettier: All files formatted correctly".to_string();
}
```

### 3B. Systemic Fix: Exit Code / Output Consistency Guard

Add `ensure_failure_visibility()` to `src/utils.rs` — a centralized guard that detects when filtered output contradicts the exit code.

```rust
/// If command failed but filtered output looks like success, append a warning.
/// This is a safety net for filter bugs — ensures LLMs see the mismatch.
pub fn ensure_failure_visibility(filtered: &mut String, exit_code: i32, raw_stderr: &str) {
    if exit_code != 0
        && (filtered.starts_with("✓")
            || filtered.contains("No issues found")
            || filtered.to_lowercase().contains("all files formatted correctly")
            || filtered.contains("All tests passed"))
    {
        filtered.push_str(&format!(
            "\n\n⚠️  Command exited with code {} but output looked clean.\n",
            exit_code
        ));
        if !raw_stderr.trim().is_empty() {
            let stderr_preview: String = raw_stderr.lines().take(5).collect::<Vec<_>>().join("\n");
            filtered.push_str(&format!("stderr: {}\n", stderr_preview));
        }
        filtered.push_str("Run raw command to verify.");
    }
}
```

**Why `stdout` not `stderr`**: LLMs primarily read stdout. A warning on stderr is invisible to the agent that caused this bug class in the first place. Appending to stdout ensures the LLM sees the contradiction.

**Placement**: Call in every filter's `run()` after filtering, before `println!`. Start with high-risk filters: prettier, vitest, cargo test, pytest, lint.

### 3C. Long-term: Filter Testing Contract

Require every filter to have **two failure case tests**:

1. **Formatting/lint failure** — unformatted files / lint errors
2. **Syntax error / crash** — tool can't even parse input

Template:
```rust
#[test]
fn test_failure_output_does_not_claim_success() {
    let failure_fixture = include_str!("../tests/fixtures/<cmd>_failure.txt");
    let filtered = filter_<cmd>(failure_fixture);
    assert!(!filtered.contains("✓"), "Failure output must not contain success marker");
    assert!(!filtered.contains("No issues found"));
    assert!(!filtered.contains("All files formatted"));
}

#[test]
fn test_syntax_error_output_shows_error() {
    let error_fixture = include_str!("../tests/fixtures/<cmd>_syntax_error.txt");
    let filtered = filter_<cmd>(error_fixture);
    assert!(!filtered.contains("✓"), "Error output must not contain success marker");
    assert!(filtered.contains("error") || filtered.contains("Error"));
}
```

---

## 4. Files to Modify

| File | Change | Priority |
|------|--------|----------|
| `src/prettier_cmd.rs` | Rewrite `filter_prettier_output()`: parse `[warn]`/`[error]` prefixes, remove hardcoded extensions, fix empty-list success logic | P0 |
| `tests/fixtures/prettier_check_failure.txt` | New fixture: real `prettier --check` output with unformatted files | P0 |
| `tests/fixtures/prettier_syntax_error.txt` | New fixture: real `prettier --check` output with syntax errors | P0 |
| `src/prettier_cmd.rs` (tests) | Add failure + syntax error tests | P0 |
| `src/utils.rs` | Add `ensure_failure_visibility()` helper | P1 |
| `src/prettier_cmd.rs` (run) | Call `ensure_failure_visibility()` before printing | P1 |
| `src/vitest_cmd.rs` (run) | Call `ensure_failure_visibility()` before printing | P2 |
| `src/cargo_cmd.rs` (run) | Call `ensure_failure_visibility()` before printing | P2 |
| `src/pytest_cmd.rs` (run) | Call `ensure_failure_visibility()` before printing | P2 |
| `src/lint_cmd.rs` (run) | Call `ensure_failure_visibility()` before printing | P2 |

---

## 5. Risk Reduction Assessment

| Mitigation | Risk Reduction | Effort |
|------------|---------------|--------|
| Fix #221 (prettier filter rewrite) | Eliminates known bug + `[error]` blind spot + extensions blind spot | Small (1-2 hours) |
| `ensure_failure_visibility()` guard | Catches ALL future filter bugs of this class | Small (1 hour) |
| Failure + syntax error fixture tests | Prevents regressions, enforces contract | Medium (1 test pair per filter) |

**Expected risk level after implementation**: LOW — the systemic guard means any new filter with this bug class will produce a visible warning rather than silent false-negative.

---

## 6. Acceptance Criteria

- [ ] `rtk prettier --check` on unformatted file shows file names (not "All files formatted")
- [ ] `rtk prettier --check` on file with syntax errors shows error details (not "All files formatted")
- [ ] Prettier filter works for all file types (no hardcoded extension list)
- [ ] Exit code is preserved (already works, verify with test)
- [ ] Failure fixture test passes for `[warn]` case
- [ ] Syntax error fixture test passes for `[error]` case
- [ ] `ensure_failure_visibility()` utility added to `src/utils.rs`
- [ ] Guard integrated into prettier `run()` function
- [ ] (Stretch) Guard integrated into vitest, cargo test, pytest, lint
- [ ] All existing tests pass
- [ ] No performance regression (<10ms startup)

---

## 7. Resolved Questions

1. **Guard scope**: Start with high-risk filters (prettier, vitest, cargo test, pytest, lint). Expand to all filters in a follow-up.
2. **Guard output channel**: Append to **stdout** (not stderr). LLMs read stdout; stderr warnings are invisible to the agent. The warning modifies the filtered string directly.
3. **Auto-fallback to raw**: Deferred. The `ensure_failure_visibility()` guard + raw stderr excerpt is sufficient for v1. Full auto-re-run adds complexity and doubles execution time.
