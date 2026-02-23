# Design Document: MEDIUM RISKS — Remaining Issues

**Date**: 2026-02-23
**Risk Level**: MEDIUM (3 issues)
**Status**: Draft

---

## Overview

Three MEDIUM RISK items remain after resolving all HIGH RISK issues (v0.22.5-v0.23.1):

| # | Issue | Summary | Module |
|---|-------|---------|--------|
| A | #186 | ESLint output duplication and count misreporting | `lint_cmd.rs` |
| B | #222 | Proxy mode buffers all output (no streaming) | `main.rs` (proxy) |
| C | #204, #229 | Clap rejects valid flags for `find` and `grep` | `main.rs`, `find_cmd.rs`, `grep_cmd.rs` |

---

## A. Output Duplication/Corruption (Issue #186)

### Problem Statement

`rtk lint` duplicates ESLint diagnostics and misreports summary counts. Example: ESLint reports "0 errors, 3 warnings" but RTK shows "3 errors, 5 warnings".

### Root Cause Analysis

**Two independent counting mechanisms that disagree:**

**Bug 1: Summary vs detail count mismatch** (lines 216-217 vs 225-232 of `lint_cmd.rs`)

The header summary sums ESLint's pre-aggregated fields:

```rust
// Lines 216-217: Use ESLint's summary fields
let total_errors: usize = results.iter().map(|r| r.error_count).sum();
let total_warnings: usize = results.iter().map(|r| r.warning_count).sum();
```

But the "Top rules" breakdown counts by iterating individual messages:

```rust
// Lines 225-232: Count by iterating messages
for result in &results {
    for msg in &result.messages {
        if let Some(rule) = &msg.rule_id {
            *by_rule.entry(rule.clone()).or_insert(0) += 1;
        }
    }
}
```

These disagree when:
- Messages have `rule_id: None` (dropped from `by_rule` but counted in ESLint's fields)
- ESLint's `errorCount`/`warningCount` don't match `messages.len()` (can happen with plugin configs)
- The per-file `count` at line 238 uses `r.messages.len()`, a third counting source

**Bug 2: No deduplication of identical messages** (lines 268-282)

Per-file rule counting iterates through all messages without deduplication:

```rust
// Lines 268-282: Inside per-file loop
for msg in &file_result.messages {
    if let Some(rule) = &msg.rule_id {
        *file_rules.entry(rule.clone()).or_insert(0) += 1;
    }
}
```

If ESLint returns duplicate messages for the same location (can happen with certain plugin configurations), they are all counted, inflating the per-file rule counts.

### Impact

- LLM receives incorrect diagnostic counts, potentially misjudging severity
- "3 errors, 5 warnings" vs "0 errors, 3 warnings" changes the urgency assessment
- Not a crash or data loss risk, but an accuracy issue

### Proposed Fix

**Use a single source of truth for all counts.** Derive everything from the `messages` vector, ignoring ESLint's summary fields (which can be stale or inconsistent).

```rust
pub fn filter_eslint_json(json_str: &str) -> Result<String> {
    let results: Vec<EslintResult> = serde_json::from_str(json_str)
        .context("Failed to parse ESLint JSON")?;

    // Single source of truth: count from messages, not ESLint summary fields
    let mut total_errors = 0usize;
    let mut total_warnings = 0usize;
    let mut by_rule: HashMap<String, usize> = HashMap::new();

    for result in &results {
        for msg in &result.messages {
            match msg.severity {
                2 => total_errors += 1,
                1 => total_warnings += 1,
                _ => {}
            }
            if let Some(rule) = &msg.rule_id {
                *by_rule.entry(rule.clone()).or_insert(0) += 1;
            }
        }
    }
    // ... use total_errors and total_warnings everywhere
}
```

**Deduplication**: Add a `HashSet<(file, line, column, rule_id)>` to skip already-seen messages:

```rust
let mut seen: HashSet<(String, u32, u32, String)> = HashSet::new();

for result in &results {
    for msg in &result.messages {
        let key = (
            result.file_path.clone(),
            msg.line,
            msg.column,
            msg.rule_id.clone().unwrap_or_default(),
        );
        if !seen.insert(key) {
            continue; // Skip duplicate
        }
        // ... count errors/warnings/rules
    }
}
```

### Files to Modify

| File | Change |
|------|--------|
| `src/lint_cmd.rs` | Rewrite counting logic: derive totals from messages, add dedup |

### Test Gaps to Fill

- Messages with `rule_id: None`
- Duplicate messages in the vector (same file, line, column, rule)
- `errorCount`/`warningCount` fields that disagree with `messages.len()`
- Empty messages array with non-zero `errorCount`

---

## B. Proxy Mode Doesn't Stream Output (Issue #222)

### Problem Statement

`rtk proxy` buffers all output until command completion using `Command::output()`. For long-running commands (deploys, log tails, builds), users see silence followed by a dump. Hard to distinguish "running" from "stuck".

### Root Cause Analysis

**File**: `src/main.rs:1498-1544`

The proxy command uses the blocking `Command::output()` API:

```rust
let output = Command::new(cmd_name.as_ref())
    .args(&cmd_args)
    .output()  // Blocks until command completes, buffers everything
    .context(format!("Failed to execute command: {}", cmd_name))?;
```

`output()` collects entire stdout/stderr into `Vec<u8>` before returning. The user sees nothing until the process exits.

### Impact

- Long-running commands appear frozen (no feedback)
- Can't distinguish "still running" from "hung process"
- Users must choose between token tracking (`rtk proxy`) and progressive output (raw command)
- Every other RTK module has the same limitation, but proxy is worst because it's the "escape hatch" for arbitrary commands

### Constraints

1. **Tracking requires byte counts**: `timer.track()` needs raw output length to compute token savings (0% for proxy, but still records metrics)
2. **Tee not used by proxy**: No tee interaction to worry about
3. **Exit code must be preserved**: Currently works, must keep working
4. **<10ms startup**: Streaming adds minimal overhead (no async runtime needed)

### Proposed Fix

Replace `Command::output()` with `Command::spawn()` + inherited stdio:

```rust
Commands::Proxy { args } => {
    use std::process::Command;

    if args.is_empty() {
        anyhow::bail!("proxy requires a command to execute\nUsage: rtk proxy <command> [args...]");
    }

    let timer = tracking::TimedExecution::start();

    let cmd_name = args[0].to_string_lossy();
    let cmd_args: Vec<String> = args[1..]
        .iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();

    if cli.verbose > 0 {
        eprintln!("Proxy mode: {} {}", cmd_name, cmd_args.join(" "));
    }

    let status = Command::new(cmd_name.as_ref())
        .args(&cmd_args)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()  // Streams directly to terminal, returns exit code only
        .context(format!("Failed to execute command: {}", cmd_name))?;

    // Track with zero token counts (can't measure what we didn't capture)
    timer.track_passthrough(
        &format!("{} {}", cmd_name, cmd_args.join(" ")),
        &format!("rtk proxy {} {}", cmd_name, cmd_args.join(" ")),
    );

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}
```

### Trade-offs

| Aspect | Buffered (current) | Streaming (proposed) |
|--------|-------------------|---------------------|
| User experience | Silence then dump | Real-time output |
| Token counting | Exact byte count (but always 0% savings) | Zero counts via `track_passthrough()` |
| Tee support | Could capture (but doesn't) | Can't capture |
| Interactive commands | Broken (stdin not inherited) | Works (stdin inherited) |
| Exit code | Preserved | Preserved |

**Key insight**: Proxy records 0% savings by definition (input = output). Switching to `track_passthrough()` with zero token counts doesn't lose useful data — it just records that the command was run and how long it took. The `rtk gain` report already handles zero-savings entries correctly.

**Bonus**: Inheriting stdin enables interactive commands (`rtk proxy vim file`, `rtk proxy less file`) which are currently broken.

### Files to Modify

| File | Change |
|------|--------|
| `src/main.rs` | Rewrite proxy block: `output()` → `status()` with inherited stdio |

### Alternative: Tee-style Streaming

If we need byte counts for proxy, use a tee approach: spawn with piped stdout/stderr, read chunks in a loop, write to terminal AND accumulate in buffer. More complex, but preserves exact token counts.

This is not recommended for v1 — the simplicity of `status()` with inherited stdio is the better trade-off.

---

## C. Argument Parsing Failures (Issues #204, #229)

### Problem Statement

RTK's Clap argument definitions for `find` and `grep` use positional arguments (`pattern: String, path: String`), which causes Clap to reject flags that belong to the underlying tool:

- `rtk find -name "*.py"` fails: Clap sees `-name` as an unknown flag
- `rtk grep -E 'pattern'` fails: Clap sees `-E` as an unknown flag

### Root Cause Analysis

**Find** (`src/main.rs:209-221`):

```rust
Find {
    pattern: String,                           // positional arg #1
    #[arg(default_value = ".")]
    path: String,                              // positional arg #2
    #[arg(short, long, default_value = "50")]
    max: usize,
    #[arg(short = 't', long, default_value = "f")]
    file_type: String,
}
```

`rtk find -name "*.py"` → Clap interprets `-name` as a flag, not a positional value. Since `-name` isn't defined in the struct, Clap errors.

**Grep** (`src/main.rs:257-281`):

```rust
Grep {
    pattern: String,                           // positional arg #1
    #[arg(default_value = ".")]
    path: String,                              // positional arg #2
    #[arg(short = 'l', long, default_value = "80")]
    max_len: usize,
    #[arg(short, long, default_value = "50")]
    max: usize,
    #[arg(short, long)]
    context_only: bool,
    #[arg(short = 't', long)]
    file_type: Option<String>,
    #[arg(short = 'n', long)]
    line_numbers: bool,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    extra_args: Vec<String>,
}
```

`rtk grep -E 'pattern'` → Clap sees `-E` before `pattern` is filled. Since `-E` isn't a defined flag, Clap errors. Note: `-n` works only because it's explicitly defined as `line_numbers: bool`.

The `extra_args` field has `trailing_var_arg = true`, but trailing var args are only captured AFTER all positional args are satisfied. Since `pattern` hasn't been consumed yet when Clap sees `-E`, the trailing var arg doesn't help.

### Why Other Commands Don't Have This Problem

Commands that work (`ls`, `tree`, `err`, `test`) have no positional args:

```rust
Ls {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,  // All arguments go here, no positional parsing
}
```

Git solved a similar problem by using a subcommand hierarchy where each subcommand takes `args: Vec<String>` with `trailing_var_arg`.

### Proposed Fix

**Remove positional arguments, use `trailing_var_arg` exclusively.** Parse `pattern` and `path` manually in the run function.

**Find refactor:**

```rust
// In main.rs
Find {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}
```

```rust
// In find_cmd.rs — manual arg parsing
pub fn run(args: &[String], verbose: u8) -> Result<()> {
    if args.is_empty() {
        anyhow::bail!("find requires a pattern\nUsage: rtk find <pattern> [path]");
    }
    let pattern = &args[0];
    let path = args.get(1).map(|s| s.as_str()).unwrap_or(".");
    // ... existing logic using pattern and path
}
```

**Grep refactor:**

```rust
// In main.rs
Grep {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}
```

```rust
// In grep_cmd.rs — extract pattern/path, pass remaining to rg
pub fn run(args: &[String], verbose: u8) -> Result<()> {
    // Separate RTK-known flags from pass-through args
    // First non-flag argument is pattern, second is path
    // All flags (known and unknown) pass through to rg

    let mut rg_args: Vec<String> = Vec::new();
    let mut positionals: Vec<String> = Vec::new();
    let mut max_results = 50usize;
    let mut max_line_len = 80usize;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--max" | "-m" => {
                if let Some(val) = args.get(i + 1) {
                    max_results = val.parse().unwrap_or(50);
                    i += 2;
                    continue;
                }
            }
            "--max-len" | "-l" => {
                if let Some(val) = args.get(i + 1) {
                    max_line_len = val.parse().unwrap_or(80);
                    i += 2;
                    continue;
                }
            }
            arg if arg.starts_with('-') => {
                rg_args.push(args[i].clone());
            }
            _ => {
                positionals.push(args[i].clone());
            }
        }
        i += 1;
    }

    let pattern = positionals.first()
        .ok_or_else(|| anyhow::anyhow!("grep requires a pattern"))?;
    let path = positionals.get(1).map(|s| s.as_str()).unwrap_or(".");

    // Build rg command with all pass-through flags
    // ...
}
```

### Impact of Refactor

| Before | After |
|--------|-------|
| `rtk find -name "*.py"` fails | `rtk find -name "*.py"` works |
| `rtk grep -E 'pattern'` fails | `rtk grep -E 'pattern'` works |
| `rtk grep -n 'pattern'` works (Clap flag) | `rtk grep -n 'pattern'` works (passed to rg) |
| `rtk find "*.py" .` works | `rtk find "*.py" .` works (backward compat) |
| `rtk grep 'pattern' src/` works | `rtk grep 'pattern' src/` works (backward compat) |

### Files to Modify

| File | Change |
|------|--------|
| `src/main.rs` | Remove positional args from Find/Grep structs, use `trailing_var_arg` |
| `src/find_cmd.rs` | Change `run()` signature to accept `args: &[String]`, parse manually |
| `src/grep_cmd.rs` | Change `run()` signature to accept `args: &[String]`, parse manually |

### Risk

LOW — The refactor simplifies parsing and makes `find`/`grep` consistent with `ls`, `tree`, and other commands. Backward compatibility is maintained since positional usage (`rtk grep 'pattern' src/`) still works through manual parsing.

---

## Priority Assessment

| Issue | Impact | Effort | Priority |
|-------|--------|--------|----------|
| C: Argument parsing (#204, #229) | Commands don't run at all | Small (2h) | P1 |
| A: Lint count misreporting (#186) | LLM gets wrong counts | Small (1-2h) | P2 |
| B: Proxy streaming (#222) | UX issue for long commands | Small (1h) | P3 |

**Rationale**: Argument parsing is P1 because the command doesn't execute at all (total failure). Lint counts are P2 because the LLM gets wrong data but the diagnostics are present. Proxy streaming is P3 because there's a trivial workaround (run the raw command).
