# Design Document: HIGH RISK 2 — Exit Code Swallowing Breaks CI/CD

**Date**: 2026-02-22
**Risk Level**: HIGH
**Status**: Revised (v2) — incorporating review feedback
**Related Issues**: [#162](https://github.com/rtk-ai/rtk/issues/162) (grep), [#185](https://github.com/rtk-ai/rtk/issues/185) (lint), [#221](https://github.com/rtk-ai/rtk/issues/221) (prettier)

---

## 1. Problem Statement

RTK wraps underlying commands but multiple modules return exit code 0 regardless of whether the underlying command succeeded or failed. This breaks CI/CD pipelines, shell control flow (`cmd && next`), and any automation that relies on exit codes to detect failures.

**The contract is simple**: `rtk <cmd>` must return the same exit code as `<cmd>`. Any deviation is a bug.

### Scope of the Problem

Full codebase audit reveals **4 modules with confirmed exit code bugs**:

#### Confirmed Bugs

**1. `grep_cmd.rs` (Issue #162 — CLOSED but NOT fixed)**

The `run()` function never checks `output.status` and always returns `Ok(())`:

- **Line 51-60**: When rg finds no matches (empty stdout), prints `"🔍 0 for 'pattern'"` and returns `Ok(())`. rg returns exit 1 for no match — RTK swallows it.
- **Line 42-45**: When rg has an error (invalid regex, missing file), the exit code from `output.status` is captured but never checked. rg returns exit 2 for errors — RTK swallows it.
- **Line 125**: Normal match case — also returns `Ok(())`, though this is correct (rg returns 0 for matches).

**Impact**: `rtk grep 'pattern' file && echo "found"` always prints "found", even when pattern doesn't exist.

**2. `runner.rs` — `run_err()` and `run_test()`**

Both functions capture exit code but never propagate it:

- **`run_err()` line 52-62**: Calculates `exit_code`, uses it for tee hint, then returns `Ok(())`. The output text shows "❌ Command failed" but the process exits 0.
- **`run_test()` line 92-103**: Same pattern — calculates `exit_code` for tee, prints test summary, returns `Ok(())`.

**Impact**: `rtk err make build` returns 0 even when `make build` fails. `rtk test npm test` returns 0 even when tests fail. CI pipelines pass when they should fail.

**3. `prisma_cmd.rs` — partial fix, wrong mechanism**

Uses `anyhow::bail!()` on failure instead of `std::process::exit()`:

- **Line 62-64**: `if !output.status.success() { anyhow::bail!("prisma generate failed: {}", stderr); }`
- `bail!` returns `Err(...)` which propagates up to `main.rs` where it prints an error message and exits 1 via the `anyhow` handler.
- This means: (a) the original exit code is lost (always becomes 1), (b) the error format differs from other RTK commands, and (c) `generate` bails BEFORE filtering — the user sees no filtered output on failure.

**Impact**: Minor — exit code is non-zero on failure, but always 1 regardless of what prisma returned (could be 2, 126, etc.). Also, `migrate` and `db push` have the same pattern.

**4. `golangci_cmd.rs` — deliberate swallow**

- **Line 84-86**: Comment says "golangci-lint returns exit code 1 when issues found (expected behavior) / Don't exit with error code in that case" — then returns `Ok(())`.
- This is intentional but **breaks the RTK contract**: if `golangci-lint` returns 1, `rtk golangci-lint` should also return 1. The user's CI pipeline may depend on that exit code.

**Impact**: `rtk golangci-lint run && echo "clean"` prints "clean" even when lint issues exist.

#### Already Fixed (for reference)

- **Issue #185** (`lint_cmd.rs`): Fixed — line 191-192 now calls `process::exit`
- **Issue #221** (`prettier_cmd.rs`): Exit code IS preserved (line 39-41); the output text bug is HIGH RISK 1

---

## 2. Impact Analysis

| Dimension | Severity | Details |
|-----------|----------|---------|
| **CI/CD** | Critical | Pipelines pass when commands fail |
| **Shell control flow** | Critical | `cmd && next` / `cmd \|\| fallback` breaks |
| **LLM agents** | High | Claude Code uses exit codes to decide next steps |
| **Discoverability** | Medium | CI failures are visible (eventually), but root cause is hard to trace to RTK |

**Worst case**: RTK wraps a test/lint command in CI → command fails → RTK exits 0 → CI reports green → broken code ships to production.

---

## 3. Proposed Solution: Systematic Exit Code Propagation

### Critical Safety Constraint: `timer.track()` Before `process::exit()`

`std::process::exit()` **immediately terminates the process** without unwinding the stack or executing `Drop` handlers. If placed before `timer.track()`, all token savings data for the command is silently lost.

This is especially harmful for failing commands — test failures, lint errors — where RTK achieves its **largest token savings** (stripping thousands of tokens of boilerplate to show only the failure summary).

**Invariant**: In every module, the call order MUST be:
```
1. println!(filtered)        // user sees output
2. timer.track(...)          // savings recorded to SQLite
3. std::process::exit(code)  // LAST — process terminates
```

Violating this order loses tracking data. The tee system (`crate::tee::tee_and_hint`) is safe — it uses `std::fs::write()` which flushes synchronously before returning.

**Validation of existing code**: All 20+ modules that currently use `process::exit()` correctly place it AFTER `timer.track()`. The proposed fixes below follow this same pattern.

### 3A. Establish the RTK Exit Code Contract

**Rule**: Every RTK command module that executes an external process MUST:
1. Filter and print output
2. Call `timer.track()` to record token savings
3. THEN call `std::process::exit(code)` when the command failed (non-zero exit)

**Standard pattern** (already used by 20+ modules):
```rust
println!("{}", filtered);
timer.track(original_cmd, rtk_cmd, &raw, &filtered);

// MUST be after timer.track()
if !output.status.success() {
    std::process::exit(output.status.code().unwrap_or(1));
}
Ok(())
```

**Error handling distinction**:
- `process::exit(code)` — for "the wrapped command failed" (preserves original exit code)
- `anyhow::bail!()` — for "RTK itself broke" (can't find binary, can't parse config, I/O error)

### 3B. Fix `grep_cmd.rs` (Issue #162)

Two cases need exit code propagation:

**Error case (exit code 2)** — invalid regex, missing file. After `output` is captured (line 45), before any filtering:

```rust
let output = rg_cmd
    .output()
    .or_else(|_| Command::new("grep").args(["-rn", pattern, path]).output())
    .context("grep/rg failed")?;

// Handle error case (rg exit 2): invalid regex, missing file
if !output.status.success() && output.status.code() != Some(1) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let msg = format!("{}", stderr.trim());
    eprintln!("{}", msg);
    // Track BEFORE exit
    timer.track(
        &format!("grep -rn '{}' {}", pattern, path),
        "rtk grep",
        &msg,
        &msg,
    );
    std::process::exit(output.status.code().unwrap_or(2));
}
```

**No-match case (exit code 1)** — rg found no matches. In the existing empty-stdout block (line 51-60), propagate after tracking:

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

rg exit codes preserved:
- 0 = matches found (no change needed)
- 1 = no matches found (new: propagated)
- 2 = error (new: propagated)

### 3C. Fix `runner.rs` — `run_err()` and `run_test()`

Both functions already compute `exit_code` and call `timer.track()`. Add `process::exit()` AFTER `timer.track()`:

**`run_err()`** — after line 61 (`timer.track`), before `Ok(())`:
```rust
    timer.track(command, "rtk run-err", &raw, &rtk);

    // MUST be after timer.track()
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
```

**`run_test()`** — after line 102 (`timer.track`), before `Ok(())`:
```rust
    timer.track(command, "rtk run-test", &raw, &summary);

    // MUST be after timer.track()
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
```

### 3D. Fix `prisma_cmd.rs` — Replace `bail!` with `process::exit`

Remove the early `bail!()` that exits before filtering. Move the failure check to the END of each function, AFTER filtering and tracking:

**For `run_generate()`:**
```rust
// Remove lines 62-64 (the early bail):
// if !output.status.success() {
//     anyhow::bail!("prisma generate failed: {}", stderr);
// }

// ... filtering and printing happens ...

println!("{}", filtered);
timer.track("prisma generate", "rtk prisma generate", &raw, &filtered);

// NEW: proper exit code propagation at the end
if !output.status.success() {
    std::process::exit(output.status.code().unwrap_or(1));
}
Ok(())
```

Apply same pattern to `run_migrate()` and `run_db_push()`. Each has the same early `bail!()` that needs replacing.

### 3E. Fix `golangci_cmd.rs` — Remove deliberate swallow

The existing code already calls `timer.track()` on lines 77-82. Replace the `Ok(())` at line 86 with exit code propagation:

```rust
    timer.track(
        &format!("golangci-lint {}", args.join(" ")),
        &format!("rtk golangci-lint {}", args.join(" ")),
        &raw,
        &filtered,
    );

    // Propagate exit code (including exit 1 for issues found)
    // Users who don't want this can configure: golangci-lint run --issues-exit-code 0
    if !output.status.success() {
        std::process::exit(output.status.code().unwrap_or(1));
    }
    Ok(())
```

---

## 4. Files to Modify

| File | Change | Priority |
|------|--------|----------|
| `src/grep_cmd.rs` | Add exit code propagation for no-match (1) and error (2), after `timer.track()` | P0 |
| `src/runner.rs` | Add `process::exit(exit_code)` after `timer.track()` in `run_err()` and `run_test()` | P0 |
| `src/prisma_cmd.rs` | Remove early `bail!()`, add `process::exit()` at end of generate/migrate/db_push | P1 |
| `src/golangci_cmd.rs` | Remove deliberate swallow, add `process::exit()` after `timer.track()` | P1 |

---

## 5. Risk Reduction Assessment

| Fix | Risk Reduction | Effort |
|-----|---------------|--------|
| grep_cmd exit codes | Fixes #162, restores rg parity | Small (30 min) |
| runner.rs exit codes | Fixes `rtk err` / `rtk test` for CI/CD | Small (30 min) |
| prisma_cmd exit codes | Preserves original exit code instead of always-1 | Small (30 min) |
| golangci_cmd exit codes | Restores CI/CD compatibility for Go linting | Small (15 min) |

**Total effort**: ~2 hours for all 4 fixes.

**Expected risk level after implementation**: LOW — all command modules will follow the established exit code propagation pattern, and CI/CD pipelines will see correct exit codes.

---

## 6. Acceptance Criteria

- [ ] `rtk grep 'nonexistent' file` returns exit 1 (not 0)
- [ ] `rtk grep '[' file` returns exit 2 (invalid regex)
- [ ] `rtk err <failing-command>` returns non-zero exit code
- [ ] `rtk test <failing-tests>` returns non-zero exit code
- [ ] `rtk prisma generate` on failure returns prisma's actual exit code (not always 1)
- [ ] `rtk golangci-lint` returns exit 1 when lint issues found
- [ ] Token tracking (`rtk gain`) records savings for ALL commands including failures
- [ ] All existing tests pass
- [ ] No performance regression

---

## 7. Resolved Questions

1. **Should `rtk grep` no-match return 1 or 0?** Follow rg behavior: exit 1 for no match. This is what Issue #162 explicitly requests, and matches both `rg` and `grep` semantics.

2. **Should `golangci_cmd` propagate exit 1?** Yes. The RTK contract says "same exit code as underlying command." Users who don't want failure-on-lint-issues can configure golangci-lint itself (`--issues-exit-code 0`). RTK should not second-guess the tool's exit code.

3. **Is `bail!()` ever appropriate in RTK command modules?** Yes, but only for RTK internal errors (can't find binary, can't parse config, I/O failure). For underlying command failures, always use `process::exit()` to preserve the original exit code.
