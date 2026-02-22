# Implementation Plan: HIGH RISK 2 — Exit Code Swallowing Breaks CI/CD

**Design Doc**: [02-high-risk-2-exit-code-swallowing.md](../design-docs/02-high-risk-2-exit-code-swallowing.md)
**Date**: 2026-02-22
**Issues**: [#162](https://github.com/rtk-ai/rtk/issues/162) (grep), [#185](https://github.com/rtk-ai/rtk/issues/185) (lint)

**Safety invariant for ALL steps**: `timer.track()` MUST be called BEFORE `std::process::exit()`. Violating this loses token savings data.

---

## Steps

### Step 1: Write failing tests for `grep_cmd.rs` exit codes (TDD Red)

Add tests to `src/grep_cmd.rs` `mod tests` that verify exit code behavior. These tests use the real `rg` binary so they validate end-to-end behavior.

**Tests to add:**

```rust
#[test]
fn test_no_match_exit_code() {
    // rg returns exit 1 for no matches
    let output = std::process::Command::new("rg")
        .args(["NONEXISTENT_PATTERN_xyz_999", "."])
        .output();
    if let Ok(out) = output {
        assert_eq!(out.status.code(), Some(1), "rg should return 1 for no match");
    }
    // If rg not installed, skip gracefully
}

#[test]
fn test_invalid_regex_exit_code() {
    // rg returns exit 2 for invalid regex
    let output = std::process::Command::new("rg")
        .args(["[", "."])
        .output();
    if let Ok(out) = output {
        assert_eq!(out.status.code(), Some(2), "rg should return 2 for invalid regex");
    }
}
```

These tests document the expected rg behavior. The actual RTK exit code propagation will be tested via integration tests (Step 8).

**Run:** `cargo test grep_cmd::tests` — tests pass (they test rg directly, not RTK).

---

### Step 2: Fix `grep_cmd.rs` — error case (exit 2)

In `src/grep_cmd.rs`, add error handling after the `output` is captured (after current line 45) and before the empty-stdout check (line 51).

**Insert after line 45** (`context("grep/rg failed")?;`):

```rust
// Handle error case (rg exit 2): invalid regex, missing file, permission error
if !output.status.success() && output.status.code() != Some(1) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let msg = stderr.trim().to_string();
    if !msg.is_empty() {
        eprintln!("{}", msg);
    }
    timer.track(
        &format!("grep -rn '{}' {}", pattern, path),
        "rtk grep",
        &msg,
        &msg,
    );
    std::process::exit(output.status.code().unwrap_or(2));
}
```

This catches exit code 2 (and any other non-0, non-1 codes) before any filtering happens.

**Run:** `cargo test grep_cmd::tests` — still passes.

---

### Step 3: Fix `grep_cmd.rs` — no-match case (exit 1)

In the existing empty-stdout block (current lines 51-61), add exit code propagation AFTER `timer.track()`.

**Change the block to:**

```rust
if stdout.trim().is_empty() {
    let msg = format!("🔍 0 for '{}'", pattern);
    println!("{}", msg);
    timer.track(
        &format!("grep -rn '{}' {}", pattern, path),
        "rtk grep",
        &raw_output,
        &msg,
    );
    // Propagate no-match exit code AFTER tracking
    if output.status.code() == Some(1) {
        std::process::exit(1);
    }
    return Ok(());
}
```

The key change: `std::process::exit(1)` added AFTER the existing `timer.track()` call, before `return Ok(())`.

**Run:** `cargo test grep_cmd::tests` — still passes.

---

### Step 4: Fix `runner.rs` — `run_err()` exit code

In `src/runner.rs`, function `run_err()`, add exit code propagation after `timer.track()` at current line 61.

**Change lines 61-62 from:**

```rust
    timer.track(command, "rtk run-err", &raw, &rtk);
    Ok(())
```

**To:**

```rust
    timer.track(command, "rtk run-err", &raw, &rtk);

    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
```

The `exit_code` variable already exists (computed at lines 52-55).

**Run:** `cargo test runner::tests` — passes.

---

### Step 5: Fix `runner.rs` — `run_test()` exit code

Same pattern in `run_test()`. Change lines 102-103 from:

```rust
    timer.track(command, "rtk run-test", &raw, &summary);
    Ok(())
```

**To:**

```rust
    timer.track(command, "rtk run-test", &raw, &summary);

    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
```

The `exit_code` variable already exists (computed at lines 92-95).

**Run:** `cargo test runner::tests` — passes.

---

### Step 6: Fix `prisma_cmd.rs` — `run_generate()`

Two changes:

**A. Remove early bail (lines 62-65):**

Delete:
```rust
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("prisma generate failed: {}", stderr);
    }
```

**B. Add exit code propagation at end (after line 74):**

Change lines 74-76 from:

```rust
    timer.track("prisma generate", "rtk prisma generate", &raw, &filtered);

    Ok(())
```

To:

```rust
    timer.track("prisma generate", "rtk prisma generate", &raw, &filtered);

    if !output.status.success() {
        std::process::exit(output.status.code().unwrap_or(1));
    }
    Ok(())
```

**Note**: With the early bail removed, `stdout`/`stderr`/`raw`/`filtered` are now computed even on failure. The filter functions are safe with empty or error output (they produce best-effort output or default messages). The user sees the filtered output, tracking is recorded, THEN exit code propagates.

**Run:** `cargo test prisma_cmd::tests` — passes.

---

### Step 7: Fix `prisma_cmd.rs` — `run_migrate()` and `run_db_push()`

Same pattern as Step 6 for both functions.

**`run_migrate()` (lines 113-116):** Remove early bail:
```rust
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("prisma migrate failed: {}", stderr);
    }
```

Add at end (after line 130, after `timer.track()`):
```rust
    if !output.status.success() {
        std::process::exit(output.status.code().unwrap_or(1));
    }
```

**`run_db_push()` (lines 151-154):** Remove early bail:
```rust
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("prisma db push failed: {}", stderr);
    }
```

Add at end (after line 163, after `timer.track()`):
```rust
    if !output.status.success() {
        std::process::exit(output.status.code().unwrap_or(1));
    }
```

**Run:** `cargo test prisma_cmd::tests` — passes.

---

### Step 8: Fix `golangci_cmd.rs` — remove deliberate swallow

In `src/golangci_cmd.rs`, replace lines 84-86.

**Change from:**

```rust
    // golangci-lint returns exit code 1 when issues found (expected behavior)
    // Don't exit with error code in that case
    Ok(())
```

**To:**

```rust
    if !output.status.success() {
        std::process::exit(output.status.code().unwrap_or(1));
    }
    Ok(())
```

The `timer.track()` call on lines 77-82 already precedes this point — safe.

**Run:** `cargo test golangci_cmd::tests` — passes.

---

### Step 9: Run full quality gate

```bash
cargo fmt --all --check
cargo clippy --all-targets
cargo test --all
```

All three must pass with zero warnings. Pay special attention to:
- No unused imports (if `anyhow::bail` is no longer used in prisma_cmd.rs, the import may need adjustment)
- No dead code warnings from removed early-bail paths

---

### Step 10: Manual verification

Test exit code propagation with real commands:

```bash
cargo build

# grep: no match → exit 1
target/debug/rtk grep 'NONEXISTENT_xyz_999' src/main.rs; echo "exit: $?"
# Expected: exit: 1

# grep: invalid regex → exit 2
target/debug/rtk grep '[' src/main.rs; echo "exit: $?"
# Expected: exit: 2

# grep: match found → exit 0
target/debug/rtk grep 'fn main' src/main.rs; echo "exit: $?"
# Expected: exit: 0

# err: failing command → non-zero
target/debug/rtk err 'false'; echo "exit: $?"
# Expected: exit: 1

# test: failing test → non-zero
target/debug/rtk test 'false'; echo "exit: $?"
# Expected: exit: 1

# Verify tracking still works for failing commands
target/debug/rtk grep 'NONEXISTENT_xyz_999' src/main.rs
target/debug/rtk gain --history | tail -5
# Expected: grep appears in history with savings recorded
```

If golangci-lint and prisma are available locally, also test:
```bash
# golangci-lint with issues → exit 1
target/debug/rtk golangci-lint run; echo "exit: $?"

# prisma generate failure → non-zero
target/debug/rtk prisma generate; echo "exit: $?"
```

---

### Step 11: Commit

Single commit with all changes:
- `src/grep_cmd.rs` — exit code propagation for no-match and error cases + tests
- `src/runner.rs` — exit code propagation in `run_err()` and `run_test()`
- `src/prisma_cmd.rs` — replaced `bail!()` with `process::exit()` in 3 functions
- `src/golangci_cmd.rs` — removed deliberate exit code swallow

---

## Summary

| Step | What | File | Validates |
|------|------|------|-----------|
| 1 | Write rg behavior tests | `grep_cmd.rs` | Document expected exit codes |
| 2 | Fix grep error case (exit 2) | `grep_cmd.rs` | Invalid regex, missing file |
| 3 | Fix grep no-match case (exit 1) | `grep_cmd.rs` | #162 fixed |
| 4 | Fix run_err() exit code | `runner.rs` | `rtk err` CI/CD safe |
| 5 | Fix run_test() exit code | `runner.rs` | `rtk test` CI/CD safe |
| 6 | Fix prisma generate | `prisma_cmd.rs` | Original exit code preserved |
| 7 | Fix prisma migrate + db push | `prisma_cmd.rs` | Original exit code preserved |
| 8 | Fix golangci-lint | `golangci_cmd.rs` | Exit 1 propagated |
| 9 | Full quality gate | all | fmt + clippy + test = clean |
| 10 | Manual verification | binary | End-to-end exit codes + tracking |
| 11 | Commit | all | All changes captured |

**Safety check in every step**: `timer.track()` always precedes `process::exit()`.
