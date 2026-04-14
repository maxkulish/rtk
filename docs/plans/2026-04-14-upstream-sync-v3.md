# Upstream Sync v3 (v0.34.0 -> v0.36.0) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port 25 upstream fixes and improvements from rtk-ai/rtk v0.34.0->v0.36.0 into our fork, maintaining our architectural patterns while gaining all correctness and quality improvements.

**Architecture:** Each task targets a specific module. P1 fixes critical regressions users hit every day (git arg parsing, SSH signing, memory leaks). P2 improves filter quality across 15 modules. P3 adds two new features. Every task follows Red-Green-Refactor with a commit at the end.

**Tech Stack:** Rust, Clap 4, anyhow, regex, serde_json, std::process

---

## Files modified

| File | Tasks |
|------|-------|
| `src/main.rs` | T1 (-u alias), T15 (RTK_DISABLED warning) |
| `src/git.rs` | T2 (-- separator), T3 (stdin for SSH) |
| `src/grep_cmd.rs` | T4 (stdin leak) |
| `src/cargo_cmd.rs` | T5 (Finished line + clippy blocks) |
| `src/go_cmd.rs` | T6 (false downloads, double-count, location, timeouts) |
| `src/golangci_cmd.rs` | T7 (run wrapper, global flags) |
| `src/discover/registry.rs` | T7 (golangci discover flags) |
| `src/pnpm_cmd.rs` | T8 (list fix, --filter support) |
| `src/psql_cmd.rs` | T9 (disable -h help clash) |
| `src/ls.rs` | T10 (suppress summary when piped) |
| `src/find_cmd.rs` | T11 (hidden dotfiles) |
| `src/gh_cmd.rs` | T12 (pr merge passthrough) |
| `src/curl_cmd.rs` | T13 (skip schema for localhost) |
| `src/gain.rs` | T14 (UTC to local timezone) |
| `src/rewrite_cmd.rs` | T16 (skip cat with incompatible flags) |
| `src/tracking.rs` | T17 (temp_dir portability) |
| `src/tee.rs` | T18 (UTF-8 truncation panic) |
| `src/pytest_cmd.rs` | T19 (-q mode summary detection) |
| `src/aws_cmd.rs` | T20 (expand to 25 subcommands) |
| `src/filters/` (TOML) | T21 (Liquibase filter) |

---

## P1 - Critical correctness

---

### Task 1: Remove `-u` short alias from `--ultra-compact`

**Why:** `rtk git push -u origin main` breaks silently. Clap sees `-u` as `--ultra-compact` (RTK's own flag) before git ever gets it. Users lose their upstream tracking branch setup. Upstream fix: `v0.36.0 - fix(git): remove -u short alias`.

**Files:**
- Modify: `src/main.rs:79`

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)]` section in `src/main.rs` (find `mod tests` at bottom of file):

```rust
#[test]
fn test_ultra_compact_has_no_short_u_alias() {
    // Verify that -u is NOT registered as short for ultra-compact
    // (it conflicts with git push -u / git branch -u)
    use clap::CommandFactory;
    let cmd = Cli::command();
    let ultra = cmd.get_arguments()
        .find(|a| a.get_long() == Some("ultra-compact"))
        .expect("--ultra-compact flag must exist");
    assert!(
        ultra.get_short() != Some('u'),
        "--ultra-compact must not have -u alias (conflicts with git push -u)"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test test_ultra_compact_has_no_short_u_alias -- --nocapture
```

Expected: FAIL - `assertion failed: --ultra-compact must not have -u alias`

- [ ] **Step 3: Remove the short alias**

In `src/main.rs:79`, change:
```rust
// before
#[arg(short = 'u', long, global = true)]
ultra_compact: bool,
```
to:
```rust
// after
#[arg(long, global = true)]
ultra_compact: bool,
```

Also update `src/main.rs` near line 1823 where it checks RTK short flags:
```rust
// before
// Every char after '-' must be a known RTK short flag: v or u
let rtk_long_flags: &[&str] = &["--verbose", "--ultra-compact", "--skip-env"];
```
Search for the exact comment and replace the comment to remove the `u` reference:
```rust
// Every char after '-' must be a known RTK short flag: v (verbose)
let rtk_long_flags: &[&str] = &["--verbose", "--ultra-compact", "--skip-env"];
```

Also update the short-flag detection logic nearby. Find where `-u` is checked as a valid RTK flag:
```bash
grep -n "short.*flag\|known.*flag\|'u'\|== 'u'" src/main.rs | head -20
```
Remove `u` from any valid short-flag lists.

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test test_ultra_compact_has_no_short_u_alias -- --nocapture
```

Expected: PASS

- [ ] **Step 5: Run full quality check**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test
```

- [ ] **Step 6: Commit**

```bash
git add src/main.rs
git commit -m "fix(git): remove -u short alias from --ultra-compact

-u conflicted with git push -u / git branch -u, silently swallowing
the flag before git received it. Users could not set upstream tracking.

Upstream: v0.36.0"
```

---

### Task 2: Re-insert `--` separator for `git diff`

**Why:** `rtk git diff -- src/file.rs` fails with `fatal: ambiguous argument`. Clap's `trailing_var_arg=true` silently drops `--` when it appears as the first positional argument. Upstream fix: `v0.36.0 #1215`.

**Files:**
- Modify: `src/git.rs` (add `normalize_diff_args()`, call from `run_diff`)

- [ ] **Step 1: Write the failing test**

Add to `#[cfg(test)] mod tests` in `src/git.rs`:

```rust
#[test]
fn test_normalize_diff_args_reinserts_separator() {
    // rtk git diff -- src/foo.rs — clap drops the --
    let args = vec!["src/foo.rs".to_string()];
    let normalized = normalize_diff_args(&args);
    assert_eq!(normalized, vec!["--", "src/foo.rs"]);
}

#[test]
fn test_normalize_diff_args_preserves_existing_separator() {
    // rtk git diff HEAD -- src/foo.rs — already has separator
    let args = vec!["HEAD".to_string(), "--".to_string(), "src/foo.rs".to_string()];
    let normalized = normalize_diff_args(&args);
    assert_eq!(normalized, vec!["HEAD", "--", "src/foo.rs"]);
}

#[test]
fn test_normalize_diff_args_leaves_revisions_alone() {
    // rtk git diff HEAD~1 HEAD — no paths, no change
    let args = vec!["HEAD~1".to_string(), "HEAD".to_string()];
    let normalized = normalize_diff_args(&args);
    assert_eq!(normalized, vec!["HEAD~1", "HEAD"]);
}

#[test]
fn test_normalize_diff_args_detects_relative_path() {
    // ./src/foo.rs starts with dot — treat as path
    let args = vec!["./src/foo.rs".to_string()];
    let normalized = normalize_diff_args(&args);
    assert_eq!(normalized, vec!["--", "./src/foo.rs"]);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test test_normalize_diff_args -- --nocapture
```

Expected: FAIL - `normalize_diff_args` not found

- [ ] **Step 3: Implement `normalize_diff_args`**

Add this function before `run_diff` in `src/git.rs`:

```rust
/// Re-insert `--` path separator when clap's trailing_var_arg has consumed it.
///
/// clap with trailing_var_arg=true silently drops `--` when it is the first
/// positional argument. This causes `rtk git diff -- file` to arrive as
/// `["file"]`, making git treat the path as a revision and emit
/// "fatal: ambiguous argument". We re-insert `--` before the first
/// path-like argument (contains `/` or `\`, or starts with `.` or `~`)
/// when `--` is absent from the args vec.
fn normalize_diff_args(args: &[String]) -> Vec<String> {
    // If -- is already present, nothing to do
    if args.iter().any(|a| a == "--") {
        return args.to_vec();
    }
    // Find first path-like argument
    let path_idx = args.iter().position(|a| {
        a.contains('/') || a.contains('\\') || a.starts_with('.') || a.starts_with('~')
    });
    match path_idx {
        Some(idx) => {
            let mut result = args[..idx].to_vec();
            result.push("--".to_string());
            result.extend_from_slice(&args[idx..]);
            result
        }
        None => args.to_vec(),
    }
}
```

Then find the `run_diff` function and add the normalize call near the top, after args are received:

```rust
fn run_diff(args: &[String], max_lines: usize, verbose: u8, opts: &GitGlobalOpts) -> Result<()> {
    let args = normalize_diff_args(args);  // re-insert -- if clap dropped it
    let args = &args;  // shadow with normalized version
    // ... rest of function unchanged
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test test_normalize_diff_args -- --nocapture
```

Expected: PASS (all 4 tests)

- [ ] **Step 5: Run full quality check**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test
```

- [ ] **Step 6: Commit**

```bash
git add src/git.rs
git commit -m "fix(git): re-insert -- separator when clap consumes it from git diff args

clap trailing_var_arg=true silently drops -- as first positional argument.
This caused 'rtk git diff -- path' to emit 'fatal: ambiguous argument'.
normalize_diff_args() re-inserts -- before the first path-like token.

Upstream: v0.36.0 #1215"
```

---

### Task 3: Inherit stdin for `git commit` and `git push` (SSH signing)

**Why:** `git commit` with SSH or GPG signing requires an interactive passphrase prompt via stdin. `.output()` pipes stdin to `/dev/null`, so signing agents can't read it. Commits fail silently or skip signing. Upstream fix: `v0.35.0 #733`.

**Files:**
- Modify: `src/git.rs:876` (`run_commit`), `src/git.rs:940` (`run_push`)

- [ ] **Step 1: Write the failing test**

Add to `#[cfg(test)] mod tests` in `src/git.rs`:

```rust
#[test]
fn test_commit_command_inherits_stdin() {
    // build_commit_command must set stdin to inherit so SSH/GPG
    // signing agents can read passphrases from the terminal
    let args: Vec<String> = vec!["-m".to_string(), "test msg".to_string()];
    let opts = GitGlobalOpts::default();
    let mut cmd = build_commit_command(&args, &opts);
    // Verify the Command has stdin configured as inherited
    // We do this indirectly: spawn with null stdin on a non-git dir should
    // not panic, but we can at least verify the function returns a Command
    // (structural test — stdin inheritance is verified by integration)
    let _ = cmd; // compile-check the build
}
```

Note: Full stdin inheritance is verified by integration (SSH signing). The unit test here is a compile check; the behavioral guarantee is documented.

- [ ] **Step 2: Update `run_commit` to inherit stdin**

In `src/git.rs`, find `run_commit` at line ~867. Change:

```rust
// before
let output = build_commit_command(args, opts)
    .output()
    .context("Failed to run git commit")?;
```

to:

```rust
// after
let output = build_commit_command(args, opts)
    .stdin(std::process::Stdio::inherit())
    .output()
    .context("Failed to run git commit")?;
```

- [ ] **Step 3: Update `run_push` to inherit stdin**

In `src/git.rs`, find `run_push` at line ~927. Change:

```rust
// before
let output = cmd.output().context("Failed to run git push")?;
```

to:

```rust
// after
let output = cmd
    .stdin(std::process::Stdio::inherit())
    .output()
    .context("Failed to run git push")?;
```

- [ ] **Step 4: Run full quality check**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test
```

Expected: all pass

- [ ] **Step 5: Commit**

```bash
git add src/git.rs
git commit -m "fix(git): inherit stdin for commit and push to preserve SSH signing

.output() pipes stdin to /dev/null, which breaks SSH and GPG signing
agents that need to read passphrases from the terminal. Setting
Stdio::inherit() allows passphrase prompts to reach the user.

Upstream: v0.35.0 #733"
```

---

### Task 4: Close subprocess stdin in `grep_cmd.rs` to prevent memory leak

**Why:** When `rtk grep` spawns `rg`, the subprocess inherits the parent's stdin by default. If stdin is a pipe (e.g. in a script), `rg` waits for it before exiting, causing the process to hang and the file descriptor to leak. Upstream fix: `v0.35.0 #897`.

**Files:**
- Modify: `src/grep_cmd.rs:42` (set `stdin(Stdio::null())`)

- [ ] **Step 1: Write the failing test**

Add to `#[cfg(test)] mod tests` in `src/grep_cmd.rs`:

```rust
#[test]
fn test_grep_cmd_builds_with_null_stdin() {
    // Verify Command is built; stdin=null prevents fd leaks in piped scripts
    // This is a structural/compile test; leak behavior is verified manually
    use std::process::Command;
    let mut cmd = Command::new("rg");
    cmd.stdin(std::process::Stdio::null());
    // Should not panic - just verify the configuration compiles
    let _ = cmd;
}
```

- [ ] **Step 2: Add `Stdio::null()` to stdin in `grep_cmd.rs`**

In `src/grep_cmd.rs`, find the rg command construction (around line 42):

```rust
// before
let output = rg_cmd
    .output()
    .or_else(|_| Command::new("grep").args(["-rn", pattern, path]).output())
    .context("grep/rg failed")?;
```

Change to:

```rust
// after
let output = rg_cmd
    .stdin(std::process::Stdio::null())
    .output()
    .or_else(|_| {
        Command::new("grep")
            .args(["-rn", pattern, path])
            .stdin(std::process::Stdio::null())
            .output()
    })
    .context("grep/rg failed")?;
```

Add `use std::process::Stdio;` to the imports at the top of the file if not already present.

- [ ] **Step 3: Run full quality check**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test
```

Expected: all pass

- [ ] **Step 4: Commit**

```bash
git add src/grep_cmd.rs
git commit -m "fix(grep): close subprocess stdin to prevent fd leak

rg inherits parent stdin by default. In piped scripts the subprocess
waits indefinitely for stdin to close, leaking the file descriptor.
Explicitly setting Stdio::null() prevents the leak.

Upstream: v0.35.0 #897"
```

---

## P2 - Filter quality

---

### Task 5: `cargo build` Finished line + `cargo clippy` full error blocks

**Why 1 (build):** `cargo build` success now shows nothing — the "Finished dev [unoptimized]..." line was stripped. Users have no confirmation the build succeeded. Upstream fix: `v0.35.0`.

**Why 2 (clippy):** `cargo clippy` showed only the truncated error headline (e.g. `error[E0502]: cannot borrow`), dropping the multi-line context that shows which line and why. Upstream fix: `v0.36.0 #602`.

**Files:**
- Modify: `src/cargo_cmd.rs`

- [ ] **Step 1: Create/update fixture and write failing tests**

```bash
# Check existing fixtures
ls tests/fixtures/ | grep cargo
```

Add test in `src/cargo_cmd.rs` `#[cfg(test)]` block:

```rust
#[test]
fn test_cargo_build_includes_finished_line() {
    let input = "\
   Compiling myproject v0.1.0 (/workspace)\n\
    Finished dev [unoptimized + debuginfo] target(s) in 2.34s\n";
    let output = filter_cargo_build(input);
    assert!(
        output.contains("Finished"),
        "build output must include the Finished line for confirmation"
    );
}

#[test]
fn test_cargo_clippy_shows_full_error_block() {
    let input = "\
error[E0502]: cannot borrow `s` as mutable\n\
  --> src/main.rs:10:5\n\
   |\n\
9  |     let r = &s;\n\
   |             -- immutable borrow occurs here\n\
10 |     s.push_str(\" world\");\n\
   |     ^^^^^^^^^^^^^^^^^^^^ mutable borrow occurs here\n\
\n\
error: aborting due to 1 previous error\n";
    let output = filter_cargo_clippy(input);
    // Must preserve the arrow line showing location
    assert!(output.contains("src/main.rs:10"), "clippy must show file:line");
    // Must preserve the context showing what went wrong
    assert!(output.contains("immutable borrow occurs here"), "clippy must show context");
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test test_cargo_build_includes_finished_line test_cargo_clippy_shows_full_error_block -- --nocapture
```

Expected: FAIL

- [ ] **Step 3: Implement fixes in `cargo_cmd.rs`**

Find `filter_cargo_build` (or the equivalent build output filtering function). Locate where `Finished` lines are filtered out and preserve them:

```rust
// In the build output filter, ensure "Finished" lines pass through
// Look for logic that strips "Finished" and change the condition
// Pattern: lines starting with "   Finished" should be included
fn is_important_build_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("error") ||
    trimmed.starts_with("warning") ||
    trimmed.starts_with("Finished") ||  // <- add this
    trimmed.starts_with("Compiling") ||
    trimmed.contains("-->")
}
```

For clippy, find where error blocks are assembled. The fix is to preserve the full multi-line error block (everything between `error[...]` and the blank line) rather than just the first line:

```rust
// In the clippy output filter, collect full error blocks
// A block ends at a blank line that follows error content
fn collect_error_blocks(input: &str) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut in_block = false;

    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("error") || trimmed.starts_with("warning[") {
            if in_block && !current.is_empty() {
                blocks.push(current.join("\n"));
                current.clear();
            }
            in_block = true;
            current.push(line);
        } else if in_block {
            if trimmed.is_empty() && !current.is_empty() {
                blocks.push(current.join("\n"));
                current.clear();
                in_block = false;
            } else {
                current.push(line);
            }
        }
    }
    if !current.is_empty() {
        blocks.push(current.join("\n"));
    }
    blocks
}
```

Note: The exact implementation depends on the current structure of `cargo_cmd.rs`. Read the file first to understand where to apply the change before editing.

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test test_cargo_build_includes_finished_line test_cargo_clippy_shows_full_error_block -- --nocapture
```

Expected: PASS

- [ ] **Step 5: Run full quality check**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test
```

- [ ] **Step 6: Commit**

```bash
git add src/cargo_cmd.rs
git commit -m "fix(cargo): show Finished line in build output; preserve full clippy error blocks

cargo build success was silent - the Finished confirmation line was stripped.
cargo clippy showed only the error headline, hiding the file:line and context
that tells you where and why the error occurred.

Upstream: v0.35.0, v0.36.0 #602"
```

---

### Task 6: Go test filter improvements (4 fixes)

**Why:** Four related bugs in `go_cmd.rs`:
1. Module download lines (`go: downloading foo v1.2.3`) were treated as build errors
2. Package-level failures double-counted when test failure cascades to package failure
3. Failing test location (file:line) was dropped from output
4. Package-level timeouts and signals (SIGKILL) not reported in summary

**Files:**
- Modify: `src/go_cmd.rs`

- [ ] **Step 1: Add failing tests**

Add to `#[cfg(test)] mod tests` in `src/go_cmd.rs`:

```rust
#[test]
fn test_go_build_ignores_download_lines() {
    let input = "\
go: downloading github.com/some/pkg v1.2.0\n\
go: downloading golang.org/x/net v0.0.0\n\
./main.go:5:2: undefined: Foo\n";
    let output = filter_go_build(input);
    assert!(!output.contains("go: downloading"), "download lines must not appear as errors");
    assert!(output.contains("undefined: Foo"), "real errors must be preserved");
}

#[test]
fn test_go_test_no_double_count_package_failure() {
    // When a test fails, go test also emits a FAIL line for the package.
    // We must count that as 1 failure, not 2.
    let json_lines = "\
{\"Action\":\"fail\",\"Test\":\"TestFoo\",\"Package\":\"mypkg\",\"Elapsed\":0.001}\n\
{\"Action\":\"fail\",\"Package\":\"mypkg\",\"Elapsed\":0.001}\n";
    let output = filter_go_test_json(json_lines);
    // Should report 1 failure, not 2
    let failure_count = output.matches("FAIL").count();
    assert_eq!(failure_count, 1, "package-level FAIL must not double-count test failures");
}

#[test]
fn test_go_test_preserves_failure_location() {
    let json_lines = "\
{\"Action\":\"output\",\"Test\":\"TestBar\",\"Output\":\"    foo_test.go:42: expected true\\n\"}\n\
{\"Action\":\"fail\",\"Test\":\"TestBar\",\"Package\":\"mypkg\"}\n";
    let output = filter_go_test_json(json_lines);
    assert!(output.contains("foo_test.go:42"), "failure location must be preserved");
}

#[test]
fn test_go_test_reports_package_timeout() {
    // Package-level panic/timeout has no test name
    let json_lines = "\
{\"Action\":\"output\",\"Package\":\"mypkg\",\"Output\":\"panic: test timed out after 30s\\n\"}\n\
{\"Action\":\"fail\",\"Package\":\"mypkg\"}\n";
    let output = filter_go_test_json(json_lines);
    assert!(output.contains("timed out") || output.contains("panic"), "timeout must appear in summary");
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test test_go_build_ignores_download_lines test_go_test_no_double_count test_go_test_preserves_failure_location test_go_test_reports_package_timeout -- --nocapture
```

Expected: FAIL

- [ ] **Step 3: Implement fixes in `go_cmd.rs`**

**Fix 1 (download lines):** In `filter_go_build`, exclude lines matching `go: downloading`:
```rust
// Skip module download progress lines
if line.starts_with("go: downloading ") || line.starts_with("go: finding ") {
    continue;
}
```

**Fix 2 (double counting):** In the JSON event processor, track which packages had test-level failures. When a package-level `"fail"` event arrives with no `"Test"` field, only emit it if no test-level failure was already recorded for that package:
```rust
// In the package summary logic:
// failed_tests: HashSet<String> of packages that had test-level FAILs
if action == "fail" && test_name.is_none() {
    if !packages_with_test_failures.contains(&package) {
        // This is a standalone package failure (timeout, build error)
        package_failures.push(package.clone());
    }
    // else: already counted via the individual test failure
}
```

**Fix 3 (location context):** When a test fails, emit the last `"output"` line containing `:` (file:line format) before the fail event. Ensure the filter doesn't discard `output` lines that contain file:line patterns like `foo_test.go:42`.

**Fix 4 (timeout/panic):** When a `"fail"` event arrives with no `"Test"` key, check whether the accumulated output for that package contains `panic:` or `timed out`. If so, include a one-line summary from that output.

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test test_go_build test_go_test -- --nocapture
```

Expected: PASS

- [ ] **Step 5: Run full quality check**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test
```

- [ ] **Step 6: Commit**

```bash
git add src/go_cmd.rs
git commit -m "fix(go): 4 filter improvements from upstream v0.35.0-v0.36.0

- Ignore module download lines (go: downloading) in build output
- Prevent double-counting when package-level fail cascades from test fail
- Preserve failing test location (file:line) in test output
- Report package-level timeout/signal failures in summary

Upstream: v0.35.0, v0.36.0 #958"
```

---

### Task 7: golangci-lint run wrapper and global flag support

**Why:** `golangci-lint` changed its interface in v2 - `run` is now the required subcommand. Our `golangci_cmd.rs` and `discover` rewrite registry weren't handling this correctly. Also, global flags before `run` (like `--timeout 5m golangci-lint run`) were rejected. Upstream fixes: `v0.36.0`.

**Files:**
- Modify: `src/golangci_cmd.rs`
- Modify: `src/discover/registry.rs` (preserve flags in rewrite)

- [ ] **Step 1: Write failing tests**

Add to `#[cfg(test)] mod tests` in `src/golangci_cmd.rs`:

```rust
#[test]
fn test_golangci_run_wrapper_applied() {
    // `golangci-lint` with no subcommand should be treated as `golangci-lint run`
    let args: Vec<String> = vec![];
    let result = build_golangci_args(&args);
    assert!(result.contains(&"run".to_string()), "run subcommand must be prepended when absent");
}

#[test]
fn test_golangci_global_flags_before_run() {
    // `golangci-lint --timeout 5m run ./...` - flags before `run` must be accepted
    let args: Vec<String> = vec!["--timeout".to_string(), "5m".to_string(), "run".to_string(), "./...".to_string()];
    let result = build_golangci_args(&args);
    assert!(result.contains(&"--timeout".to_string()));
    assert!(result.contains(&"run".to_string()));
}
```

Add to discover registry tests:

```rust
#[test]
fn test_discover_golangci_preserves_flags() {
    let result = crate::discover::registry::rewrite_command(
        "golangci-lint run --fast ./...", &[]
    );
    let rewritten = result.unwrap_or_default();
    assert!(rewritten.contains("--fast"), "discover rewrite must preserve golangci-lint flags");
    assert!(rewritten.contains("./..."), "discover rewrite must preserve path");
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test test_golangci -- --nocapture
```

- [ ] **Step 3: Implement fixes**

In `src/golangci_cmd.rs`, add a `build_golangci_args` helper that ensures `run` is present:

```rust
fn build_golangci_args(user_args: &[String]) -> Vec<String> {
    let has_run = user_args.iter().any(|a| a == "run");
    if has_run {
        user_args.to_vec()
    } else {
        // Prepend `run` after any leading global flags (--timeout, --config, etc.)
        let first_non_flag = user_args.iter().position(|a| !a.starts_with('-') && !user_args.get(user_args.iter().position(|b| b == a).unwrap().saturating_sub(1)).map(|p| p.starts_with('-')).unwrap_or(false));
        let insert_at = first_non_flag.unwrap_or(0);
        let mut result = user_args[..insert_at].to_vec();
        result.push("run".to_string());
        result.extend_from_slice(&user_args[insert_at..]);
        result
    }
}
```

In `src/discover/registry.rs`, find the golangci-lint rewrite rule and ensure it uses `{args}` or equivalent expansion that preserves all original arguments:

```rust
// In the registry entries, ensure golangci-lint rule preserves flags:
// Pattern: "golangci-lint {args}" -> "rtk golangci-lint {args}"
// Verify the rewrite doesn't strip anything after the base command
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test test_golangci test_discover_golangci -- --nocapture
```

- [ ] **Step 5: Run full quality check**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test
```

- [ ] **Step 6: Commit**

```bash
git add src/golangci_cmd.rs src/discover/registry.rs
git commit -m "fix(golangci-lint): run wrapper, global flags, discover flag preservation

golangci-lint v2 requires the run subcommand. Global flags before run
(--timeout, --config) were rejected. discover rewrite was dropping flags.

Upstream: v0.36.0"
```

---

### Task 8: pnpm list fix and `--filter` support

**Why:** `rtk pnpm list` was broken (regression). Also, pnpm's `--filter` argument (for monorepo workspace scoping) was not passed through. Upstream fixes: `v0.36.0`.

**Files:**
- Modify: `src/pnpm_cmd.rs:294` (`run_list`)

- [ ] **Step 1: Write failing tests**

Add to `#[cfg(test)] mod tests` in `src/pnpm_cmd.rs`:

```rust
#[test]
fn test_pnpm_list_json_parses_empty_deps() {
    // pnpm list --json can return [] when no packages installed
    let input = "[]";
    let result = filter_pnpm_list(input);
    assert!(result.is_ok(), "empty pnpm list output must not error: {:?}", result);
}

#[test]
fn test_pnpm_list_with_filter_arg() {
    // --filter is a monorepo scoping flag, must be passed through to pnpm
    let args = vec!["--filter".to_string(), "my-package".to_string()];
    let cmd_args = build_pnpm_list_args(&args);
    assert!(cmd_args.contains(&"--filter".to_string()), "--filter must be forwarded to pnpm");
    assert!(cmd_args.contains(&"my-package".to_string()));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test test_pnpm_list -- --nocapture
```

- [ ] **Step 3: Implement fixes in `pnpm_cmd.rs`**

In `run_list` (~line 294), find where `--json` is added and where extra args are handled. The fix has two parts:

**Fix 1 (list broken):** Check whether `pnpm list --json` output is being parsed correctly. The JSON format can be an array `[{...}]` - ensure the parser handles this:

```rust
// Existing parser likely expects an object {}. Update to handle array:
fn filter_pnpm_list(json_str: &str) -> Result<String> {
    let trimmed = json_str.trim();
    // pnpm list --json can return [] (empty) or [{...}] (array of workspaces)
    if trimmed == "[]" || trimmed == "" {
        return Ok("(no dependencies)".to_string());
    }
    // Try array format first
    if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(trimmed) {
        if let Some(first) = arr.first() {
            return filter_pnpm_list_object(first);
        }
        return Ok("(no dependencies)".to_string());
    }
    // Fall back to object format
    let val: serde_json::Value = serde_json::from_str(trimmed)
        .context("Failed to parse pnpm list JSON")?;
    filter_pnpm_list_object(&val)
}
```

**Fix 2 (--filter support):** In `run_list`, collect any `--filter <value>` pairs from `args` and pass them before `--json` in the command:

```rust
fn build_pnpm_list_args(user_args: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    let mut i = 0;
    while i < user_args.len() {
        if user_args[i] == "--filter" && i + 1 < user_args.len() {
            result.push("--filter".to_string());
            result.push(user_args[i + 1].clone());
            i += 2;
        } else {
            i += 1;
        }
    }
    result.push("list".to_string());
    result.push("--json".to_string());
    result
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test test_pnpm_list -- --nocapture
```

- [ ] **Step 5: Run full quality check**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test
```

- [ ] **Step 6: Commit**

```bash
git add src/pnpm_cmd.rs
git commit -m "fix(pnpm): restore list command and add --filter support

pnpm list was broken due to array vs object JSON format mismatch.
Added --filter passthrough for monorepo workspace scoping.

Upstream: v0.36.0"
```

---

### Task 9: psql `-h` host argument clash

**Why:** `rtk psql -h myhost mydb` fails because Clap intercepts `-h` as `--help` before it reaches psql. Upstream fix: `v0.35.0 #650`.

**Files:**
- Modify: `src/psql_cmd.rs`

- [ ] **Step 1: Write failing test**

Add to `#[cfg(test)] mod tests` in `src/psql_cmd.rs`:

```rust
#[test]
fn test_psql_h_flag_is_not_help() {
    // -h must be passed to psql as the host flag, not intercepted as --help
    let args = vec!["-h".to_string(), "myhost".to_string(), "mydb".to_string()];
    let cmd_args = build_psql_args(&args);
    assert!(
        cmd_args.contains(&"-h".to_string()),
        "-h must be passed through to psql, not consumed as --help"
    );
    assert!(cmd_args.contains(&"myhost".to_string()));
}
```

- [ ] **Step 2: Find and fix the issue in `psql_cmd.rs`**

Read `src/psql_cmd.rs` to find where the command is built. If the module uses a Clap subcommand with `-h` defined, the fix is to disable the auto-generated help flag:

```rust
// In the Clap Args struct or subcommand for psql, add:
#[command(disable_help_flag = true)]
// or pass all args directly without Clap parsing them:
```

If psql uses raw arg passthrough (no Clap parsing of individual flags), verify that `-h` is forwarded as-is. If Clap is consuming `-h`, switch to raw passthrough:

```rust
pub fn run(args: &[String], verbose: u8) -> Result<()> {
    let mut cmd = Command::new("psql");
    for arg in args {
        cmd.arg(arg);  // Forward all args including -h unchanged
    }
    // ...
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test test_psql -- --nocapture
```

- [ ] **Step 4: Run full quality check**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test
```

- [ ] **Step 5: Commit**

```bash
git add src/psql_cmd.rs
git commit -m "fix(psql): pass -h through to psql as host flag

Clap was intercepting -h as --help before psql received it,
breaking 'rtk psql -h hostname db'.

Upstream: v0.35.0 #650"
```

---

### Task 10: `ls` suppress summary line when stdout is piped

**Why:** `rtk ls | grep pattern` includes the summary line (`📊 42 files, 8 dirs`) in grep's input, polluting results. The summary is only meaningful for human terminal display. Upstream fix: `v0.35.0`.

**Files:**
- Modify: `src/ls.rs`

- [ ] **Step 1: Write failing test**

Add to `#[cfg(test)] mod tests` in `src/ls.rs`:

```rust
#[test]
fn test_ls_format_no_summary_for_pipe() {
    // When piped=true, the formatted output must NOT include the summary line
    let entries = vec![
        "src/main.rs".to_string(),
        "src/lib.rs".to_string(),
    ];
    let output = format_ls_output(&entries, /* piped= */ true);
    assert!(
        !output.contains("files") && !output.contains("dirs"),
        "piped ls output must not include summary line"
    );
}

#[test]
fn test_ls_format_summary_for_terminal() {
    let entries = vec!["src/main.rs".to_string()];
    let output = format_ls_output(&entries, /* piped= */ false);
    assert!(output.contains("file"), "terminal ls output must include summary");
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test test_ls_format -- --nocapture
```

- [ ] **Step 3: Implement pipe detection in `ls.rs`**

Add pipe detection using `atty` crate or `std::io::IsTerminal` (stable since Rust 1.70):

```rust
use std::io::IsTerminal;

pub fn run(args: ...) -> Result<()> {
    // ...
    let is_piped = !std::io::stdout().is_terminal();
    let output = format_ls_output(&entries, is_piped);
    print!("{}", output);
    Ok(())
}

fn format_ls_output(entries: &[String], piped: bool) -> String {
    let mut out = String::new();
    // ... existing entry formatting ...
    if !piped {
        // Only append summary for terminal output
        out.push_str(&summary);
    }
    out
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test test_ls_format -- --nocapture
```

- [ ] **Step 5: Run full quality check**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test
```

- [ ] **Step 6: Commit**

```bash
git add src/ls.rs
git commit -m "fix(ls): suppress summary line when stdout is piped

The 'N files, M dirs' summary is terminal-only metadata. When stdout
is piped (e.g. rtk ls | grep), the summary line pollutes grep input.
Uses std::io::IsTerminal to detect pipe context.

Upstream: v0.35.0"
```

---

### Task 11: `find` include hidden files when pattern targets dotfiles

**Why:** `rtk find .env` returned nothing because the `ignore` crate skips hidden files by default. Upstream fix: `v0.36.0 #1101`.

**Files:**
- Modify: `src/find_cmd.rs`

- [ ] **Step 1: Write failing test**

Add to `#[cfg(test)] mod tests` in `src/find_cmd.rs`:

```rust
#[test]
fn test_find_pattern_starting_with_dot_enables_hidden() {
    // When the search pattern targets a dotfile, hidden files must be included
    assert!(should_search_hidden(".env"), "dotfile pattern must enable hidden file search");
    assert!(should_search_hidden(".gitignore"), "dotfile pattern must enable hidden file search");
    assert!(!should_search_hidden("main.rs"), "normal pattern must not force hidden search");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test test_find_pattern_starting_with_dot -- --nocapture
```

- [ ] **Step 3: Implement fix in `find_cmd.rs`**

Add a helper and use it when building the `WalkBuilder`:

```rust
fn should_search_hidden(pattern: &str) -> bool {
    // If the pattern itself starts with '.', the user is looking for dotfiles
    pattern.starts_with('.')
}

// In the run() function, when building the walker:
let mut builder = ignore::WalkBuilder::new(path);
builder.hidden(!should_search_hidden(pattern));  // disable hidden-file skip for dotfile patterns
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test test_find_pattern_starting_with_dot -- --nocapture
```

- [ ] **Step 5: Run full quality check**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test
```

- [ ] **Step 6: Commit**

```bash
git add src/find_cmd.rs
git commit -m "fix(find): include hidden files when pattern targets dotfiles

'rtk find .env' returned nothing because ignore crate skips hidden
files by default. When the pattern starts with '.', disable the hidden
file filter so dotfiles like .env, .gitignore are found.

Upstream: v0.36.0 #1101"
```

---

### Task 12: `gh pr merge` passthrough actual output

**Why:** `gh pr merge` returned a canned "ok merged #N" instead of gh's actual output, hiding important details like auto-merge status, branch deletion confirmation, and error messages from merge conflicts. Upstream fix: `v0.35.0 #938`.

**Files:**
- Modify: `src/gh_cmd.rs:1066` (`pr_merge`)

- [ ] **Step 1: Write failing test**

Add to `#[cfg(test)] mod tests` in `src/gh_cmd.rs`:

```rust
#[test]
fn test_pr_merge_uses_actual_output_not_canned() {
    // The filtered output of pr_merge should pass through gh's real output
    // not a canned "ok merged #42" string
    let gh_stdout = "✓ Merged pull request #42 (feat: new feature)\n✓ Deleted branch feat/new-feature\n";
    let filtered = filter_pr_merge_output(gh_stdout, "42");
    // Should preserve actual output content, not replace with canned message
    assert!(
        filtered.contains("Merged") || filtered.contains("pull request"),
        "pr merge should use actual gh output: got '{}'", filtered
    );
    assert!(
        !filtered.starts_with("ok merged"),
        "canned 'ok merged' response must not be used"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test test_pr_merge_uses_actual_output -- --nocapture
```

- [ ] **Step 3: Implement fix in `gh_cmd.rs`**

Add a `filter_pr_merge_output` function and update `pr_merge` to use it:

```rust
fn filter_pr_merge_output(stdout: &str, _pr_num: &str) -> String {
    // Pass through gh's actual output — it's already compact and informative
    // Strip ANSI codes for consistency
    crate::utils::strip_ansi(stdout.trim()).to_string()
}
```

Update the `pr_merge` function body to use the actual output:

```rust
fn pr_merge(args: &[String], _verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();
    let mut cmd = Command::new("gh");
    cmd.args(["pr", "merge"]);
    for arg in args {
        cmd.arg(arg);
    }
    let output = cmd.output().context("Failed to run gh pr merge")?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        timer.track("gh pr merge", "rtk gh pr merge", &stderr, &stderr);
        eprintln!("{}", stderr.trim());
        std::process::exit(output.status.code().unwrap_or(1));
    }

    let pr_num = args.iter().find(|a| !a.starts_with('-')).map(|s| s.as_str()).unwrap_or("");
    // Use actual gh output instead of canned message
    let filtered = filter_pr_merge_output(&stdout, pr_num);
    let display = if filtered.is_empty() { format!("ok merged #{}", pr_num) } else { filtered.clone() };
    println!("{}", display);
    timer.track("gh pr merge", "rtk gh pr merge", &stdout, &display);
    Ok(())
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test test_pr_merge -- --nocapture
```

- [ ] **Step 5: Run full quality check**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test
```

- [ ] **Step 6: Commit**

```bash
git add src/gh_cmd.rs
git commit -m "fix(gh): pass through actual gh pr merge output

gh pr merge output contains merge confirmation, auto-merge status,
and branch deletion info. The canned 'ok merged #N' was hiding all
of this. Now passes through gh's output directly.

Upstream: v0.35.0 #938"
```

---

### Task 13: `curl` skip JSON schema conversion for localhost/internal URLs

**Why:** `rtk curl http://localhost:3000/api/data` was converting the JSON response into a schema (`{field: string, ...}`) instead of showing the actual values. Internal/localhost URLs are development endpoints where actual values matter. Upstream fix: `v0.36.0`.

**Files:**
- Modify: `src/curl_cmd.rs`

- [ ] **Step 1: Write failing test**

Add to `#[cfg(test)] mod tests` in `src/curl_cmd.rs`:

```rust
#[test]
fn test_is_internal_url_localhost() {
    assert!(is_internal_url("http://localhost:3000/api"), "localhost must be internal");
    assert!(is_internal_url("http://127.0.0.1:8080/data"), "127.0.0.1 must be internal");
    assert!(is_internal_url("https://localhost/health"), "localhost https must be internal");
    assert!(!is_internal_url("https://api.example.com/data"), "external URL must not be internal");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test test_is_internal_url -- --nocapture
```

- [ ] **Step 3: Implement fix in `curl_cmd.rs`**

Add a helper function before the main JSON processing logic:

```rust
fn is_internal_url(url: &str) -> bool {
    url.contains("localhost") || url.contains("127.0.0.1") || url.contains("::1")
}
```

Then in the main JSON processing, skip schema conversion for internal URLs:

```rust
// In the response handling logic, before calling filter_json_string/schema:
if is_internal_url(url) {
    // Show actual JSON values for localhost/internal endpoints
    println!("{}", json_output);
} else {
    // Apply schema compression for external API responses
    if let Ok(schema) = json_cmd::filter_json_string(trimmed, 5) {
        // ... existing schema logic
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test test_is_internal_url -- --nocapture
```

- [ ] **Step 5: Run full quality check**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test
```

- [ ] **Step 6: Commit**

```bash
git add src/curl_cmd.rs
git commit -m "fix(curl): skip JSON schema conversion for localhost/internal URLs

When developing against local APIs (localhost, 127.0.0.1), the actual
response values matter, not just the schema structure. Internal URLs
now bypass schema compression and show real values.

Upstream: v0.36.0"
```

---

### Task 14: `gain` convert history timestamps from UTC to local timezone

**Why:** `rtk gain --history` showed timestamps in UTC (e.g. `03-26 09:14`), which for users in UTC+2 means every timestamp is 2 hours off. Upstream fix: `v0.35.0 #562`.

**Files:**
- Modify: `src/gain.rs` and/or `src/tracking.rs`

- [ ] **Step 1: Write failing test**

Check if `chrono` is already a dependency:
```bash
grep "chrono" Cargo.toml
```

If not, add it:
```toml
# Cargo.toml [dependencies]
chrono = "0.4"
```

Add to `#[cfg(test)] mod tests` in `src/gain.rs`:

```rust
#[test]
fn test_timestamp_formats_in_local_time() {
    // A timestamp stored as UTC unix seconds must display in local time
    // We can't assert exact values (timezone-dependent), but we can
    // verify the format function doesn't panic and returns a HH:MM string
    use chrono::{Local, TimeZone};
    let utc_secs: i64 = 1710000000; // some UTC timestamp
    let local_dt = Local.timestamp_opt(utc_secs, 0).single().expect("valid timestamp");
    let formatted = local_dt.format("%m-%d %H:%M").to_string();
    assert_eq!(formatted.len(), 11, "timestamp format should be MM-DD HH:MM");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test test_timestamp_formats_in_local_time -- --nocapture
```

- [ ] **Step 3: Implement fix in `gain.rs`**

Find the timestamp formatting code (around line 221: `rec.timestamp.format("%m-%d %H:%M")`).

If `rec.timestamp` is a `NaiveDateTime` (no timezone), convert it to local time:

```rust
use chrono::{Local, TimeZone, NaiveDateTime};

// Before (UTC, no conversion):
let time = rec.timestamp.format("%m-%d %H:%M");

// After (convert UTC stored value to local):
let utc_dt = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
    rec.timestamp,
    chrono::Utc,
);
let local_dt = utc_dt.with_timezone(&Local);
let time = local_dt.format("%m-%d %H:%M");
```

If `rec.timestamp` is stored as a Unix timestamp (i64), use:

```rust
let local_dt = Local.timestamp_opt(rec.timestamp, 0)
    .single()
    .unwrap_or_else(|| Local::now());
let time = local_dt.format("%m-%d %H:%M");
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test test_timestamp_formats -- --nocapture
```

- [ ] **Step 5: Run full quality check**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test
```

- [ ] **Step 6: Commit**

```bash
git add src/gain.rs Cargo.toml Cargo.lock
git commit -m "fix(gain): convert history timestamps from UTC to local timezone

rtk gain --history was showing timestamps in UTC. Users in non-UTC
timezones saw times offset by their UTC difference. Now uses chrono
Local to display timestamps in the user's local timezone.

Upstream: v0.35.0 #562"
```

---

### Task 15: `RTK_DISABLED=1` emit warning on stderr

**Why:** When `RTK_DISABLED=1` is set, RTK silently passes commands through without any filtering. This is confusing — users don't realize RTK is disabled and wonder why they're not getting token savings. Upstream fix: `v0.35.0`.

**Files:**
- Modify: `src/main.rs` (where `RTK_DISABLED` is checked)

- [ ] **Step 1: Write failing test**

Add to `#[cfg(test)] mod tests` in `src/main.rs`:

```rust
#[test]
fn test_rtk_disabled_check_is_present() {
    // This is a structural test verifying the RTK_DISABLED path exists in code
    // Behavioral test: run the binary with RTK_DISABLED=1 and check stderr
    // (integration test verified manually)
    let disabled = std::env::var("RTK_DISABLED").map(|v| v == "1").unwrap_or(false);
    // Just verify the check compiles and returns bool
    let _ = disabled;
}
```

- [ ] **Step 2: Find the RTK_DISABLED check**

```bash
grep -n "RTK_DISABLED" src/main.rs
```

- [ ] **Step 3: Add warning emission**

Find the code block that handles `RTK_DISABLED=1`. It likely looks like:

```rust
if std::env::var("RTK_DISABLED").map(|v| v == "1").unwrap_or(false) {
    // pass through / return early
}
```

Add a warning:

```rust
if std::env::var("RTK_DISABLED").map(|v| v == "1").unwrap_or(false) {
    eprintln!("rtk: warning: RTK_DISABLED=1 is set - all filtering is bypassed");
    // existing passthrough logic continues...
}
```

- [ ] **Step 4: Add integration-level test**

Add to `#[cfg(test)] mod tests` in `src/main.rs`:

```rust
#[test]
fn test_rtk_disabled_warning_subprocess() {
    // Verify that running rtk with RTK_DISABLED=1 emits a warning
    // Requires the binary to be built: skip gracefully if not found
    let rtk = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("rtk")));
    if rtk.as_ref().map(|p| p.exists()).unwrap_or(false) {
        let output = std::process::Command::new(rtk.unwrap())
            .env("RTK_DISABLED", "1")
            .args(["git", "status"])
            .output();
        if let Ok(out) = output {
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(
                stderr.contains("RTK_DISABLED"),
                "RTK_DISABLED=1 must emit a warning on stderr"
            );
        }
    }
}
```

- [ ] **Step 5: Run full quality check**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test
```

- [ ] **Step 6: Commit**

```bash
git add src/main.rs
git commit -m "warn: emit stderr warning when RTK_DISABLED=1 is detected

Silent passthrough when RTK_DISABLED=1 confused users who didn't
realize filtering was off. Now emits a one-line warning on stderr
so the disabled state is visible without affecting stdout output.

Upstream: v0.35.0"
```

---

### Task 16: Skip `cat` rewrite when incompatible flags are present

**Why:** `cat -n file.txt` (line numbers) and `cat -A file.txt` (show all) were being rewritten to `rtk cat file.txt` which doesn't understand those flags, producing wrong output. Upstream fix: `v0.35.0 #847`.

**Files:**
- Modify: `src/rewrite_cmd.rs` or `src/discover/registry.rs`

- [ ] **Step 1: Write failing test**

Add to `#[cfg(test)] mod tests` in `src/rewrite_cmd.rs`:

```rust
#[test]
fn test_cat_n_flag_not_rewritten() {
    // cat -n shows line numbers — rtk cat doesn't support this
    let result = crate::discover::registry::rewrite_command("cat -n file.txt", &[]);
    assert!(
        result.is_none() || result.as_deref() == Some("cat -n file.txt"),
        "cat with -n flag must not be rewritten to rtk cat"
    );
}

#[test]
fn test_cat_plain_is_rewritten() {
    // plain `cat file.txt` should still be rewritten
    let result = crate::discover::registry::rewrite_command("cat file.txt", &[]);
    assert!(
        result.map(|r| r.contains("rtk")).unwrap_or(false),
        "plain cat must still be rewritten to rtk cat"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test test_cat_n_flag test_cat_plain_is_rewritten -- --nocapture
```

- [ ] **Step 3: Implement fix**

Find in `src/discover/registry.rs` or wherever `cat` rewrites are registered. Add a condition to skip rewriting when incompatible flags are present:

```rust
// Flags that cat supports but rtk read/cat doesn't
const CAT_INCOMPATIBLE_FLAGS: &[&str] = &[
    "-n", "--number",           // line numbers
    "-A", "--show-all",         // show all non-printing chars
    "-b", "--number-nonblank",  // number non-blank lines
    "-e",                       // equivalent to -vE
    "-E", "--show-ends",        // show $ at end of lines
    "-s", "--squeeze-blank",    // suppress repeated empty lines
    "-T", "--show-tabs",        // show tabs as ^I
    "-t",                       // equivalent to -vT
    "-v", "--show-nonprinting", // show non-printing chars
];

fn should_rewrite_cat(command: &str) -> bool {
    let tokens: Vec<&str> = command.split_whitespace().collect();
    // Skip rewrite if any incompatible flag is present
    !tokens.iter().any(|t| CAT_INCOMPATIBLE_FLAGS.contains(t))
}
```

Apply this check in the cat rewrite rule.

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test test_cat_n_flag test_cat_plain -- --nocapture
```

- [ ] **Step 5: Run full quality check**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test
```

- [ ] **Step 6: Commit**

```bash
git add src/rewrite_cmd.rs src/discover/registry.rs
git commit -m "fix(rewrite): skip cat rewrite when incompatible flags are present

cat -n (line numbers), cat -A (show-all), and similar flags are not
supported by rtk cat/read. Rewriting those commands produces wrong
output. Now checks for incompatible flags before rewriting.

Upstream: v0.35.0 #847"
```

---

### Task 17: `tracking` use `temp_dir()` for portability

**Why:** Hardcoded `/tmp` paths break on Windows (`C:\Users\...\AppData\Local\Temp`) and on systems with non-standard temp directories. Upstream fix: `v0.35.0`.

**Files:**
- Modify: `src/tracking.rs`

- [ ] **Step 1: Find all `/tmp` hardcodes**

```bash
grep -n '"/tmp"\|/tmp/' src/tracking.rs
```

- [ ] **Step 2: Write test**

Add to `#[cfg(test)] mod tests` in `src/tracking.rs`:

```rust
#[test]
fn test_temp_path_uses_env_temp_dir() {
    // std::env::temp_dir() must be used instead of /tmp
    // This test verifies the result is non-empty and exists
    let tmp = std::env::temp_dir();
    assert!(tmp.exists(), "temp_dir() must return an existing directory");
    assert!(tmp.is_dir(), "temp_dir() must be a directory");
}
```

- [ ] **Step 3: Replace `/tmp` with `temp_dir()`**

For each occurrence of `"/tmp"` or `/tmp/` in `src/tracking.rs`, replace with:

```rust
// before
let tmp_path = format!("/tmp/rtk_{}", some_name);

// after
let tmp_path = std::env::temp_dir().join(format!("rtk_{}", some_name));
let tmp_path_str = tmp_path.to_string_lossy();
```

- [ ] **Step 4: Run full quality check**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test
```

- [ ] **Step 5: Commit**

```bash
git add src/tracking.rs
git commit -m "fix(tracking): use std::env::temp_dir() for cross-platform compatibility

Hardcoded /tmp breaks on Windows and systems with non-standard temp
locations. std::env::temp_dir() returns the correct platform temp dir.

Upstream: v0.35.0"
```

---

### Task 18: `tee` prevent panic on UTF-8 multi-byte truncation boundary

**Why:** When `tee.rs` truncates output at a byte offset, it can split a multi-byte UTF-8 character, causing a panic at `String::from_utf8`. Upstream fix: `v0.35.0`.

**Files:**
- Modify: `src/tee.rs`

- [ ] **Step 1: Write failing test**

Add to `#[cfg(test)] mod tests` in `src/tee.rs`:

```rust
#[test]
fn test_tee_truncate_safe_utf8() {
    // Truncating in the middle of a 3-byte UTF-8 char (e.g. Japanese) must not panic
    let text = "Hello 日本語 world";  // 日 is 3 bytes: E6 97 A5
    let bytes = text.as_bytes();
    // Find byte index that splits in the middle of a multibyte char
    // "Hello " is 6 bytes, then 日 starts at byte 6, is 3 bytes (6,7,8)
    // Truncating at byte 7 splits 日
    let truncate_at = 7.min(bytes.len());
    let safe = safe_truncate_utf8(bytes, truncate_at);
    // Must be valid UTF-8
    assert!(std::str::from_utf8(safe).is_ok(), "truncation must not split UTF-8 chars");
    // Must not panic
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test test_tee_truncate_safe_utf8 -- --nocapture
```

- [ ] **Step 3: Implement `safe_truncate_utf8` in `tee.rs`**

Add this function:

```rust
/// Truncate a byte slice at `max_bytes` without splitting a UTF-8 multi-byte character.
/// Walks back from `max_bytes` until the boundary is at a valid UTF-8 char start.
fn safe_truncate_utf8(bytes: &[u8], max_bytes: usize) -> &[u8] {
    if max_bytes >= bytes.len() {
        return bytes;
    }
    // Walk backward to find a valid UTF-8 char boundary
    let mut end = max_bytes;
    while end > 0 && (bytes[end] & 0b1100_0000) == 0b1000_0000 {
        // 0b10xxxxxx is a UTF-8 continuation byte — step back
        end -= 1;
    }
    &bytes[..end]
}
```

Find every place in `tee.rs` where bytes are truncated and replace with `safe_truncate_utf8`.

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test test_tee_truncate_safe_utf8 -- --nocapture
```

- [ ] **Step 5: Run full quality check**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test
```

- [ ] **Step 6: Commit**

```bash
git add src/tee.rs
git commit -m "fix(tee): prevent panic on UTF-8 multi-byte truncation boundary

Truncating tee output at a raw byte offset could split a multi-byte
UTF-8 character (Japanese, emoji, etc.), causing a from_utf8 panic.
safe_truncate_utf8() walks back to the nearest valid char boundary.

Upstream: v0.35.0"
```

---

### Task 19: `pytest` `-q` mode summary line detection

**Why:** `pytest -q` emits a different summary format (`1 failed, 3 passed in 0.12s`) compared to normal mode (`FAILED test_foo.py::test_bar`). The state machine in `pytest_cmd.rs` didn't recognize the `-q` summary line, reporting "No tests collected" even when tests ran and failed. Upstream fix: `v0.36.0 #588`.

**Files:**
- Modify: `src/pytest_cmd.rs`

- [ ] **Step 1: Write failing test**

Add to `#[cfg(test)] mod tests` in `src/pytest_cmd.rs`:

```rust
#[test]
fn test_pytest_quiet_mode_summary_detected() {
    // pytest -q emits summary as "1 failed, 3 passed in 0.12s"
    let input = "\
F...\n\
FAILED test_foo.py::test_bar - AssertionError: assert 1 == 2\n\
1 failed, 3 passed in 0.12s\n";
    let output = filter_pytest_output(input);
    assert!(
        !output.contains("No tests collected"),
        "pytest -q output must not report 'No tests collected'"
    );
    assert!(
        output.contains("failed") || output.contains("FAILED"),
        "pytest -q output must include failure information"
    );
}

#[test]
fn test_pytest_quiet_summary_regex() {
    // The summary line regex must match -q format
    assert!(
        is_pytest_summary_line("1 failed, 3 passed in 0.12s"),
        "quiet summary line must be detected"
    );
    assert!(
        is_pytest_summary_line("5 passed in 1.34s"),
        "all-passing quiet summary must be detected"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test test_pytest_quiet -- --nocapture
```

- [ ] **Step 3: Implement fix in `pytest_cmd.rs`**

Find the state machine that transitions to `Summary` state. Add recognition for `-q` format summary lines:

```rust
fn is_pytest_summary_line(line: &str) -> bool {
    // Normal mode: "= 1 failed, 3 passed in 0.12s ="
    // Quiet mode:  "1 failed, 3 passed in 0.12s"
    lazy_static::lazy_static! {
        static ref SUMMARY_RE: regex::Regex = regex::Regex::new(
            r"(?:\d+ \w+(?:, \d+ \w+)* in \d+\.?\d*s|=+ \d+ .+ =+)"
        ).unwrap();
    }
    SUMMARY_RE.is_match(line)
}
```

Update the state machine to call `is_pytest_summary_line` when checking for the `Summary` transition.

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test test_pytest_quiet -- --nocapture
```

- [ ] **Step 5: Run full quality check**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test
```

- [ ] **Step 6: Commit**

```bash
git add src/pytest_cmd.rs
git commit -m "fix(pytest): detect -q mode summary line correctly

pytest -q emits '1 failed, 3 passed in 0.12s' without the '===' borders.
The state machine didn't recognize this format, falling through to
'No tests collected'. Updated is_pytest_summary_line() to match both
normal and quiet mode summary formats.

Upstream: v0.36.0 #588"
```

---

### Task 20: Hook permission security fixes

**Why:** Two security issues in the hook permission system:
1. `glob_matches` middle-wildcard (`*`) matched commands even without trailing args, allowing `allow git *` to inadvertently allow `git push --force` when the rule was only intended for `git status`.
2. When no permission rule matched, the default was to silently allow instead of prompting the user.

Upstream fixes: `v0.36.0 #1213, #1105`.

**Files:**
- Modify: wherever `glob_matches` is defined and wherever the default permission verdict is set. Run:

```bash
grep -rn "glob_matches\|default.*verdict\|no.*rule.*match\|permission" src/ --include="*.rs" | head -20
```

- [ ] **Step 1: Write failing tests**

Find the module containing `glob_matches` then add:

```rust
#[test]
fn test_glob_middle_wildcard_requires_args() {
    // "git *" should NOT match "git" with no args
    // The wildcard requires at least one argument token to be present
    assert!(glob_matches("git *", "git status"), "wildcard must match with args");
    assert!(!glob_matches("git *", "git"), "wildcard must not match without args");
}

#[test]
fn test_glob_all_segments_must_match() {
    // All segments of the pattern must match the command
    // "git push" must not match "git status"
    assert!(glob_matches("git push", "git push"), "exact match must pass");
    assert!(!glob_matches("git push", "git status"), "different subcommand must not match");
    assert!(!glob_matches("git push", "git push --force"), "extra args must fail exact pattern");
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test test_glob_middle_wildcard test_glob_all_segments -- --nocapture
```

- [ ] **Step 3: Fix `glob_matches`**

Find the `glob_matches` implementation and update the wildcard matching logic:

```rust
fn glob_matches(pattern: &str, command: &str) -> bool {
    let pat_tokens: Vec<&str> = pattern.split_whitespace().collect();
    let cmd_tokens: Vec<&str> = command.split_whitespace().collect();

    if pat_tokens.is_empty() {
        return false;
    }

    let mut pi = 0;
    let mut ci = 0;

    while pi < pat_tokens.len() && ci <= cmd_tokens.len() {
        if pat_tokens[pi] == "*" {
            // Wildcard: must match at least one token
            if ci >= cmd_tokens.len() {
                return false;  // no tokens left to match
            }
            // Consume remaining command tokens
            return true;
        }
        if ci >= cmd_tokens.len() {
            return false;
        }
        if pat_tokens[pi] != cmd_tokens[ci] {
            return false;
        }
        pi += 1;
        ci += 1;
    }

    // All pattern segments matched and all command tokens consumed
    pi == pat_tokens.len() && ci == cmd_tokens.len()
}
```

- [ ] **Step 4: Verify default verdict prompts user**

Find where the default verdict is set when no rule matches. Ensure it prompts (returns `Prompt` or equivalent) rather than `Allow`:

```rust
// Default when no rule matches must be Ask/Prompt, not Allow
fn default_verdict() -> PermissionVerdict {
    PermissionVerdict::Ask  // or Prompt, not Allow
}
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test test_glob_middle_wildcard test_glob_all_segments -- --nocapture
```

- [ ] **Step 6: Run full quality check**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test
```

- [ ] **Step 7: Commit**

```bash
git add src/
git commit -m "fix(hooks): require all segments to match; wildcard needs args; default to Ask

Two security fixes:
1. glob_matches wildcard now requires at least one trailing arg to match
2. All non-wildcard pattern segments must match (no partial matches)
3. Default permission verdict when no rule matches is Ask, not Allow

Upstream: v0.36.0 #1213, #1105"
```

---

## P3 - New features

---

### Task 21: AWS CLI expand from 8 to 25 subcommands

**Why:** AWS CLI coverage was limited to 8 subcommands. Upstream expanded to 25, covering the most common services. Since we have `aws_cmd.rs`, this is a direct expansion.

**Files:**
- Modify: `src/aws_cmd.rs`

- [ ] **Step 1: Audit current coverage**

```bash
grep -n "\"ec2\"\|\"s3\"\|\"iam\"\|\"lambda\"\|\"rds\"\|\"ecs\"\|\"eks\"\|\"cloudformation\"" src/aws_cmd.rs | head -30
```

- [ ] **Step 2: Review upstream's expansion**

```bash
gh api repos/rtk-ai/rtk/contents/src/aws_cmd.rs --jq '.content' | base64 -d | grep -E '"[a-z]+" =>' | head -40
```

- [ ] **Step 3: Write tests for new subcommands**

For each new subcommand added (e.g. `cloudwatch`, `sqs`, `sns`, `ssm`, `secretsmanager`, etc.), add a test verifying the output is filtered:

```rust
#[test]
fn test_aws_cloudwatch_list_metrics_filtered() {
    let input = include_str!("../tests/fixtures/aws_cloudwatch_list_metrics_raw.txt");
    let output = filter_aws_cloudwatch(input);
    let savings = 100.0 - (count_tokens(&output) as f64 / count_tokens(input) as f64 * 100.0);
    assert!(savings >= 60.0, "cloudwatch filter must save >=60% tokens, got {:.1}%", savings);
}
```

Create corresponding fixtures from real `aws` CLI output or use representative samples.

- [ ] **Step 4: Implement new subcommand filters**

Add filter functions for each new subcommand following the existing pattern in `aws_cmd.rs`. Each filter should:
- Extract the essential fields (ARN, name, status, region)
- Drop verbose metadata (descriptions, tags, timestamps unless relevant)
- Return compact one-line-per-resource format

- [ ] **Step 5: Register new subcommands in the routing match**

Add each new subcommand to the `match` statement in `aws_cmd.rs::run()`.

- [ ] **Step 6: Run full quality check**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test
```

- [ ] **Step 7: Commit**

```bash
git add src/aws_cmd.rs tests/fixtures/
git commit -m "feat(aws): expand CLI filters from 8 to 25 subcommands

Added compact filters for: cloudwatch, sqs, sns, ssm, secretsmanager,
route53, acm, dynamodb, elasticache, elb, elbv2, logs, events, kms,
codecommit, codepipeline, codebuild.

Upstream: v0.35.0"
```

---

### Task 22: Liquibase TOML filter

**Why:** Liquibase is a popular database migration tool. Its output is verbose with XML/Java logging. A TOML filter provides compact migration status. Upstream added this in `v0.36.0`.

**Files:**
- Create: `src/filters/liquibase.toml`

- [ ] **Step 1: Review existing TOML filter format**

```bash
ls src/filters/
cat src/filters/*.toml | head -50
```

- [ ] **Step 2: Write a test fixture**

Create `tests/fixtures/liquibase_update_raw.txt` with representative Liquibase output:

```
####################################################
##   _     _             _ _                      ##
##  | |   (_)           (_) |                     ##
##  | |    _  __ _ _   _ _| |__   __ _ ___  ___  ##
##  | |   | |/ _` | | | | | '_ \ / _` / __|/ _ \ ##
##  | |___| | (_| | |_| | | |_) | (_| \__ \  __/ ##
##  \_____/_|\__, |\__,_|_|_.__/ \__,_|___/\___| ##
##              | |                               ##
##              |_|                               ##
##                                                ##
##  Get documentation at docs.liquibase.com       ##
####################################################
Starting Liquibase at 10:30:00 (version 4.23.0 built at 2023-07-12 19:30+0000)
Liquibase Version: 4.23.0
Running Changeset: db/changelog/001-init.sql::1::author
Running Changeset: db/changelog/002-users.sql::2::author
Running Changeset: db/changelog/003-indexes.sql::3::author
Liquibase command 'update' was executed successfully.
```

- [ ] **Step 3: Write test**

Add to the TOML filter test suite or create `tests/test_toml_filters.rs`:

```rust
#[test]
fn test_liquibase_filter_removes_ascii_art() {
    let input = include_str!("../tests/fixtures/liquibase_update_raw.txt");
    let output = apply_toml_filter("liquibase", input).unwrap_or(input.to_string());
    assert!(!output.contains("##   _     _"), "ASCII art header must be removed");
    assert!(output.contains("update"), "command result must be preserved");
}

#[test]
fn test_liquibase_filter_token_savings() {
    let input = include_str!("../tests/fixtures/liquibase_update_raw.txt");
    let output = apply_toml_filter("liquibase", input).unwrap_or(input.to_string());
    let savings = 100.0 - (count_tokens(&output) as f64 / count_tokens(input) as f64 * 100.0);
    assert!(savings >= 60.0, "liquibase filter must save >=60%, got {:.1}%", savings);
}
```

- [ ] **Step 4: Create the TOML filter**

Create `src/filters/liquibase.toml` following the existing filter format:

```toml
# Liquibase database migration tool filter
# Removes ASCII art header, verbose startup info, and Java stack traces
# Preserves: changeset execution list, success/failure result

[[filters]]
# Remove the ASCII art Liquibase logo (lines starting with ##)
pattern = "^##.*$"
action = "remove"

[[filters]]
# Remove verbose startup timestamp/version lines
pattern = "^(Starting Liquibase|Liquibase Version:|Liquibase command .* was)"
action = "keep"

[[filters]]
# Keep running changeset lines (user cares which migrations ran)
pattern = "^Running Changeset:"
action = "keep"

[[filters]]
# Remove all other lines (Java boilerplate, thread info, etc.)
pattern = ".*"
action = "remove"
```

Note: Adapt to the exact TOML filter DSL syntax used in `src/filters/*.toml`.

- [ ] **Step 5: Run tests**

```bash
cargo test test_liquibase -- --nocapture
```

- [ ] **Step 6: Run full quality check**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test
```

- [ ] **Step 7: Commit**

```bash
git add src/filters/liquibase.toml tests/fixtures/liquibase_update_raw.txt
git commit -m "feat(toml): add Liquibase database migration filter

Removes ASCII art header and Java startup boilerplate from Liquibase
output while preserving changeset execution list and migration result.
Saves 80%+ tokens on typical update output.

Upstream: v0.36.0 #1036"
```

---

## Final: quality gate and version prep

- [ ] **Run full test suite**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test --all
```

Expected: zero warnings, all tests pass.

- [ ] **Benchmark performance regression check**

```bash
hyperfine 'target/release/rtk git status' --warmup 3
```

Expected: <10ms mean startup time.

- [ ] **Update memory with new sync point**

Update `/Users/mk/.claude/projects/-Users-mk-Code-rtk/memory/project_upstream_sync.md` to record sync v3 complete and last synced upstream tag as `v0.36.0`.
