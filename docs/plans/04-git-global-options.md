# Fix: Git global options before subcommands (Issue #228)

## Context

`rtk git --no-pager diff` fails because Clap rejects `--no-pager` before reaching the git subcommand parser. The `Commands::Git` variant only has `command: GitCommands` with no fields for global git options. PR #99 (upstream, merged) fixed this at the **hook level** only; the RTK binary itself never learned about git global options.

**Affected commands**: Any `rtk git <global-opt> <subcommand>`, e.g.:
- `rtk git --no-pager diff`
- `rtk git -C /path status`
- `rtk git --no-optional-locks status`

## Plan

### 1. Add `GitGlobalOpts` struct to `src/git.rs`

```rust
#[derive(Debug, Clone, Default)]
pub struct GitGlobalOpts {
    pub no_pager: bool,
    pub no_optional_locks: bool,
    pub bare: bool,
    pub literal_pathspecs: bool,
    pub dir: Vec<String>,       // -C <path> (repeatable)
    pub config: Vec<String>,    // -c key=val (repeatable)
    pub git_dir: Option<String>,
    pub work_tree: Option<String>,
}
```

### 2. Add `git_cmd()` helper in `src/git.rs`

Factory function that builds `Command::new("git")` with global args prepended. Replaces all ~27 direct `Command::new("git")` calls in the module.

### 3. Thread `opts: &GitGlobalOpts` through all functions in `src/git.rs`

- Update `run()` and `run_passthrough()` public signatures
- Update all 13 private `run_*` functions + `build_commit_command`
- Replace every `Command::new("git")` with `git_cmd(opts)`

### 4. Add Clap fields to `Commands::Git` in `src/main.rs`

Add 8 fields before `#[command(subcommand)]`:
- `--no-pager`, `--no-optional-locks`, `--bare`, `--literal-pathspecs` (bool flags)
- `-C <PATH>` (Vec), `-c <KEY=VALUE>` (Vec)
- `--git-dir <PATH>`, `--work-tree <PATH>` (Option)

### 5. Update match block in `src/main.rs`

Destructure new fields, construct `GitGlobalOpts`, pass to `git::run()` / `git::run_passthrough()`.

### 6. Fix 3 existing test destructurings (add `..`)

### 7. Add tests

- 6 Clap parsing tests in `src/main.rs` (no-pager, -C, -c repeatable, combined, git-dir/work-tree, backward compat)
- 6 `git_cmd()` unit tests in `src/git.rs` (default, no-pager, single dir, multi dir, config, all opts)

## Files to modify

| File | Changes |
|------|---------|
| `src/git.rs` | Add struct, helper, update ~27 Command calls, update 15 function signatures, add 6 tests |
| `src/main.rs` | Add 8 fields to Git variant, update match block, fix 3 tests, add 6 tests |

## Verification

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test
rtk git --no-pager diff          # should work (was failing)
rtk git -C /tmp status           # should work
rtk git --no-pager log -5        # should work
rtk git status                   # backward compat (still works)
```
