# Implementation Plan: Clap Parse-Failure Fallback

**Design Doc**: [04-medium-risks.md](../design-docs/04-medium-risks.md) (Section C)
**Date**: 2026-02-23
**Issues**: [#204](https://github.com/rtk-ai/rtk/issues/204), [#229](https://github.com/rtk-ai/rtk/issues/229)

**Safety invariant**: Fallback must NOT trigger on `--help`, `--version`, or bare `rtk` invocation.

---

## Context

RTK uses `Cli::parse()` which exits with a Clap error on unrecognized arguments. Commands like `rtk find -name "*.py"` or `rtk grep -E 'pattern'` fail instead of running the underlying command. The fix: switch to `try_parse()` and automatically fall back to raw command execution on parse failure.

RTK already has this pattern at the subcommand level (`external_subcommand` + `run_passthrough()` in git.rs, cargo_cmd.rs, pnpm_cmd.rs). This lifts it to the top level.

---

## Steps

### Step 1: Add `ErrorKind` import

Line 52 of `src/main.rs` — add `use clap::error::ErrorKind;`

### Step 2: Replace `Cli::parse()` with `try_parse()` + fallback

Lines 890-891 — capture raw args, try parse, fallback on error.

### Step 3: Add `should_fallback()` function

Checks Clap error kind — only fallback on `InvalidSubcommand`, `UnknownArgument`, `InvalidValue`, `NoEquals`. Respects `RTK_NO_FALLBACK` env var opt-out.

### Step 4: Add `strip_rtk_flags()` helper

Handles `rtk -v make build` — strips leading RTK flags (`-v`/`-vv`, `-u`, `--verbose`, `--ultra-compact`, `--skip-env`) so fallback runs `make build`.

### Step 5: Add `run_fallback()` function

Follows `run_passthrough()` pattern (git.rs:1368): streaming via `.status()`, `track_passthrough()` for metrics, exit code preservation, stderr hint.

### Step 6: Add unit tests

10 tests: unknown subcommand, unknown flag, help, version, bare rtk, env opt-out, known commands, strip_rtk_flags variants.

### Step 7: Add smoke tests

3 assertions in `scripts/test-all.sh`: `rtk echo hello`, output check, exit code.

### Step 8: Quality gate + manual verification

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test
rtk echo hello
rtk find . -name "*.rs"
rtk --help
rtk -v echo test
```

---

## Files to modify

| File | Change |
|------|--------|
| `src/main.rs` | Import, `try_parse` + match, 3 functions, 10 tests |
| `scripts/test-all.sh` | 3 smoke test assertions |

## Summary

| Step | What | Validates |
|------|------|-----------|
| 1 | Add import | Compile |
| 2 | Replace parse() with try_parse() | Fallback wired in |
| 3 | should_fallback() | Correct error classification |
| 4 | strip_rtk_flags() | RTK flags don't leak to fallback |
| 5 | run_fallback() | Raw command executes with streaming + tracking |
| 6 | Unit tests | Logic correctness |
| 7 | Smoke tests | End-to-end verification |
| 8 | Quality gate | fmt + clippy + test = clean |
