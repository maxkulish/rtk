# Upstream Sync Plan: v0.24.0 -> v0.29.0

**Created**: 2026-03-14
**Our fork**: maxkulish/rtk v0.24.0 (51 modules, 55k LOC, binary 4.8MB)
**Upstream**: rtk-ai/rtk v0.29.0 (confirmed via `git describe --tags upstream/master`)
**Delta**: 172 files changed, +8492 / -21956 lines between HEAD and upstream/master
**Strategy**: Port from upstream as reference implementation, adapt to our codebase

---

## Pre-Flight Checklist

Run before starting any porting work:

- [ ] Verify upstream: `git fetch upstream --tags && git describe --tags upstream/master`
- [ ] Create sync branch: `git checkout -b sync/upstream-v0.29.0`
- [ ] Audit dependencies: `cargo outdated && cargo audit`
- [ ] Baseline binary size: `cargo build --release && ls -lh target/release/rtk`
- [ ] Baseline startup time: `hyperfine 'target/release/rtk git status' --warmup 3`
- [ ] Baseline test count: `cargo test 2>&1 | grep 'test result'`

---

## Branch Strategy

```
master (v0.24.0)
  |
  +-- sync/upstream-p1-bugfixes
  |     |-- commit per fix group (git, gh, other)
  |     |-- tag: v0.25.0-alpha.1 (after all P1 done)
  |     +-- merge to master, release v0.25.0
  |
  +-- sync/upstream-p2-features
  |     |-- commit per feature
  |     |-- tag: v0.26.0-alpha.1
  |     +-- merge to master, release v0.26.0
  |
  +-- sync/upstream-p3-toml-dsl
        |-- P3.1: engine + 14 filters
        |-- P3.2: user config + 4 filters
        |-- P3.3: 15 more filters
        |-- P3.4: rewrite engine
        |-- tag: v0.28.0-alpha.1
        +-- If succeeds: merge to master, release v0.28.0
        +-- If fails: release v0.26.0 with P1+P2 only, defer TOML
```

**Commit message format**: `fix(sync): <description> (upstream #NNN)` or `feat(sync): ...`
This keeps Release Please from creating a massive version bump per cherry-pick.

---

## Conflict Resolution Protocol

Our fork diverges significantly from upstream. **Do not blindly cherry-pick.**

1. **Read upstream diff first**: `git show <upstream-sha> -- <file>` to understand the change
2. **Compare with our version**: `git diff HEAD upstream/master -- <file>` to see full divergence
3. **For heavily modified files** (git.rs, gh_cmd.rs, filter.rs, utils.rs, lint_cmd.rs):
   - Treat upstream as *reference implementation*
   - Manually reimplement the fix into our fork's architecture
   - Preserve our fork-specific additions (GitGlobalOpts, ensure_failure_visibility, etc.)
4. **For new files** (aws_cmd.rs, psql_cmd.rs, gt_cmd.rs, toml_filter.rs):
   - Copy upstream file directly
   - Adapt imports and module registration
   - Scrub telemetry calls (see Telemetry Scrubbing section)
5. **Document semantic conflicts**: when upstream behavior differs from our fork assumptions

### Key divergence points (verified via `git diff`)

| File | Divergence | Approach |
|------|-----------|----------|
| `src/git.rs` | We have `GitGlobalOpts` struct, upstream uses `global_args: &[String]` | Keep our struct, port bug fix logic |
| `src/gh_cmd.rs` | We have `run_api()` with json_cmd filter, upstream passthrough | Port upstream's `extract_identifier_and_extra_args()`, adopt passthrough for api |
| `src/utils.rs` | We have `ensure_failure_visibility()`, upstream removed it | Keep ours, port upstream's `join_with_overflow()` and `truncate_iso_date()` |
| `src/filter.rs` | Minimal diff - just `Language::Data` variant missing | Clean cherry-pick possible |
| `src/lint_cmd.rs` | We have mypy delegation, upstream has it too | Verify no conflict, port prefix stripping |

---

## Telemetry Scrubbing

Upstream v0.26.0+ added anonymous telemetry (`ureq`, `sha2`, `hostname`, `flate2`).
We are NOT porting telemetry. When porting code that touches telemetry:

1. **Do not add** `sha2`, `ureq`, `hostname`, `flate2` to Cargo.toml
2. **Remove/stub** any `telemetry::` calls in ported code
3. **Skip** `src/telemetry.rs` entirely
4. **Check** ported modules for `use crate::telemetry` imports

---

## Phase 1 - Bug Fixes (16 items)

High value, low risk. Isolated changes to existing modules.
Squash into 3 commits: git fixes, gh fixes, other fixes.

### P1.1 - Git fixes

- [ ] **P1.1a** - git: propagate exit codes in push/pull/fetch/stash/worktree (#234)
  - File: `src/git.rs`
  - Upstream commit: `5cfaecc`
  - We have exit code propagation in runner/golangci/prisma but NOT in git push/pull/fetch/stash/worktree
  - **Approach**: Reference impl - adapt to our `GitGlobalOpts` pattern
  - **Test fixture needed**: N/A (exit code test, not output)

- [ ] **P1.1b** - git commit -am, --amend and other flags (#327/#360)
  - File: `src/git.rs`
  - Upstream commit: `409aed6`
  - `git commit -am "msg"` currently fails or misroutes
  - **Approach**: Reference impl - our `Commit { messages }` variant differs from upstream's `Commit`
  - **Test fixture needed**: `git commit -am` routing test

- [ ] **P1.1c** - git branch creation silently swallowed by list mode (#194)
  - File: `src/git.rs`
  - Upstream commit: `88dc752`
  - `rtk git branch newbranch` gets treated as list instead of create
  - **Approach**: Reference impl - check how branch args are parsed
  - **Test fixture needed**: `git branch <name>` routing test

- [ ] **P1.1d** - git log --oneline no longer silently truncated to 10 entries (#461/#478)
  - File: `src/git.rs`
  - Upstream commit: from `d0396da`
  - Only inject -10 limit when RTK applies its own compact format; respect user's --oneline/--pretty/--format
  - **Approach**: Reference impl - check limit injection logic
  - **Test fixture needed**: git log with --oneline flag

- [ ] **P1.1e** - git: support multiple -m flags in git commit
  - File: `src/git.rs`
  - Upstream commit: `c18553a`
  - `git commit -m "title" -m "body"` should work
  - **Approach**: Reference impl - our `Commit { messages: Vec<String> }` may already handle this, verify

### P1.2 - GitHub CLI fixes

- [ ] **P1.2a** - gh pr edit/comment pass correct subcommand (#332)
  - File: `src/gh_cmd.rs`
  - Upstream commit: `799f085`
  - **Approach**: Upstream changed `pr_action()` to pass full args including subcommand. Port this fix.
  - **Test fixture needed**: verify `gh pr comment 123 -b "text"` routes correctly

- [ ] **P1.2b** - pass through -R/--repo flag in gh view commands (#328)
  - File: `src/gh_cmd.rs`
  - Upstream commit: `0a1bcb0`
  - **Approach**: Port `extract_identifier_and_extra_args()` function from upstream. This replaces our simple `args[0]` identifier extraction with proper flag-aware parsing.
  - **Test fixture needed**: `gh pr view 123 -R owner/repo` and `gh pr view -R owner/repo 123`

- [ ] **P1.2c** - reduce gh diff / git diff / gh api truncation (#354/#370)
  - File: `src/gh_cmd.rs`, `src/git.rs`
  - Upstream commit: `e356c12`
  - **Approach**: Increase `compact_diff` limit from 100 to 500 lines. Change `run_api()` to passthrough.
  - **Breaking change**: `rtk gh api` will now pass through raw JSON instead of compacting. This is intentional - compaction destroyed values and forced re-fetching.

- [ ] **P1.2d** - smart markdown body filter for gh issue/pr view (#188/#214)
  - File: `src/gh_cmd.rs`
  - Upstream commit: `4208015`
  - **Approach**: Reference impl - check `filter_markdown_segment()` changes

- [ ] **P1.2e** - gh run view --job flag loses its value (#416/#477)
  - File: `src/gh_cmd.rs`
  - Upstream commit: from `d0396da`
  - Add --job and --attempt to flags_with_value in `extract_identifier_and_extra_args()`
  - **Approach**: Included in P1.2b's `extract_identifier_and_extra_args()` port

### P1.3 - Other module fixes

- [ ] **P1.3a** - playwright: fix JSON parser for real output format (#193)
  - File: `src/playwright_cmd.rs`
  - Upstream commit: `4eb6cf4`
  - **Approach**: Compare upstream parser with ours, adapt
  - **Test fixture needed**: real Playwright JSON output

- [ ] **P1.3b** - preserve `--` separator for cargo commands (#326)
  - File: `src/cargo_cmd.rs` or `src/main.rs`
  - Upstream commit: `45f9344`
  - `rtk cargo test -- --nocapture` must pass `--` through
  - **Approach**: Check Clap config and arg forwarding

- [ ] **P1.3c** - strip npx/bunx/pnpm prefixes in lint detection (#186/#366)
  - File: `src/lint_cmd.rs`
  - Upstream commit: `27b35d8`
  - **Approach**: Reference impl - our lint_cmd.rs has mypy delegation, check for conflicts
  - **Test fixture needed**: lint output with npx/pnpm prefix

- [ ] **P1.3d** - grep: translate BRE `\|` alternation, strip -r for rg (#206)
  - File: `src/grep_cmd.rs`
  - Upstream commit: `70d1b04`
  - **Approach**: Add regex translation for `\|` -> `|` and strip `-r` flag
  - **Test fixture needed**: grep with `\|` pattern

- [ ] **P1.3e** - rtk read no longer corrupts JSON/YAML/TOML files (#464/#479)
  - File: `src/filter.rs`
  - Upstream commit: from `d0396da`
  - Add `Language::Data` variant for JSON, YAML, TOML, XML, Markdown, CSV
  - `packages/*` in package.json was treated as block comment start (`/*`)
  - **Approach**: Clean port - upstream diff is small and our filter.rs is close
  - **Test fixture needed**: package.json with `packages/*` glob

- [ ] **P1.3f** - npm routing fix - install != run install (#470)
  - File: `src/npm_cmd.rs`
  - Upstream commit: from `d0396da`
  - `rtk npm install` was executing as `npm run install`
  - **Approach**: Check our npm_cmd.rs routing logic, fix subcommand detection

### P1 Quality Gates

After all Phase 1 items:
- [ ] `cargo fmt --all && cargo clippy --all-targets && cargo test`
- [ ] Binary size still < 5MB: `cargo build --release && ls -lh target/release/rtk`
- [ ] Startup time still < 10ms: `hyperfine 'target/release/rtk git status' --warmup 3`
- [ ] Manual test: `rtk git log --oneline -5`, `rtk gh pr view 1`, `rtk cargo test -- --nocapture`
- [ ] Squash into commit: `fix(sync): port 16 upstream bug fixes (v0.24.0..v0.27.2)`

---

## Phase 2 - New Features (10 items)

Medium value features. Some require new files, some extend existing modules.
One commit per feature.

### P2.1 - New command modules

- [ ] **P2.1a** - AWS CLI module (`src/aws_cmd.rs`)
  - Upstream commit: `b934466` (#216)
  - New file + main.rs registration
  - Token-optimized output for aws s3, ec2, ecs, etc.
  - **Security note**: Must preserve error states, permission warnings, secret exposure warnings in output. Snapshot tests must cover error cases.

- [ ] **P2.1b** - psql module (`src/psql_cmd.rs`)
  - Upstream commit: `b934466` (#216)
  - Same PR as AWS, new file + main.rs registration
  - **Security note**: Must preserve connection errors, permission denied, and query errors. Never truncate error output from database operations.

- [ ] **P2.1c** - Graphite CLI support (`src/gt_cmd.rs`)
  - Upstream commit: `7fbc4ef` (#290)
  - New file + main.rs registration

### P2.2 - Existing module enhancements

- [ ] **P2.2a** - `rtk rewrite` - single source of truth for hook rewrites
  - Upstream commit: `f447a3d` (#241)
  - New `src/rewrite.rs` + main.rs registration
  - Replaces shell-script-based rewrite logic with Rust
  - **Telemetry check**: verify no telemetry calls in this module

- [ ] **P2.2b** - find: accept native flags (-name, -type, etc.) (#211)
  - File: `src/find_cmd.rs`
  - Upstream commit: `7ac5bc4`

- [ ] **P2.2c** - Colored gain dashboard with efficiency meter (#129)
  - File: `src/gain.rs`
  - Upstream commit: `606b86e`
  - Better UX for `rtk gain` output

- [ ] **P2.2d** - Stream proxy output while running (#268)
  - File: `src/main.rs` (proxy handler)
  - Upstream commit: `884e37e`
  - Currently buffers entire output; should stream
  - **Note**: Must remain single-threaded (no async/tokio). Use `std::io::BufReader` line-by-line.

- [ ] **P2.2e** - Python lint dispatcher + universal format (#100)
  - File: `src/lint_cmd.rs`, `src/format_cmd.rs`
  - Upstream commit: `4cae6b6`
  - **Conflict risk**: lint_cmd.rs is heavily modified in our fork. Use reference impl approach.

- [ ] **P2.2f** - curl JSON size guard (#297) + exclude_commands config (#243)
  - File: `src/curl_cmd.rs`, `src/config.rs`
  - Upstream commit: `a8d6106`

- [ ] **P2.2g** - rtk init: upsert_rtk_block for idempotent CLAUDE.md management (#123)
  - File: `src/init.rs`
  - Upstream commit: `356c0d6`

### P2 Quality Gates

After all Phase 2 items:
- [ ] `cargo fmt --all && cargo clippy --all-targets && cargo test`
- [ ] Binary size still < 5MB
- [ ] Startup time still < 10ms
- [ ] Manual test each new command: `rtk aws`, `rtk psql`, `rtk gt`
- [ ] Update CLAUDE.md module table with new modules
- [ ] Update ARCHITECTURE.md module count
- [ ] Squash related items, use `feat(sync):` prefix

### P2 Decision Gate (before Phase 3)

Evaluate before starting Phase 3:
- [ ] Is our binary size still under 4.5MB? (need ~0.5MB headroom for TOML deps)
- [ ] Do we need custom user filters? If no users request this, consider skipping P3.2
- [ ] Is the full 47-filter set needed? Consider the minimal port alternative (see below)

---

## Phase 3 - TOML Filter DSL

High value, high risk. Fundamental architecture change.
Port from upstream in order: Part 1 -> Part 2 -> Part 3.

### Performance & Size Constraints (CRITICAL)

**Verified upstream design decisions:**

1. **Built-in TOML filters are compiled into the binary** via `build.rs`:
   - `build.rs` concatenates all `src/filters/*.toml` into one file at compile time
   - Embedded via `include_str!(concat!(env!("OUT_DIR"), "/builtin_filters.toml"))`
   - **Zero file I/O on startup** for built-in filters - constraint satisfied

2. **User overrides read from disk only when command matches**:
   - Priority: `.rtk/filters.toml` (project) > `~/.config/rtk/filters.toml` (global) > built-in
   - File I/O only happens when a TOML-filtered command is invoked, not on every startup
   - **Lazy loading** - constraint satisfied

3. **New dependencies required**:
   - `toml = "0.8"` (already in our Cargo.toml)
   - `build.rs` needs `toml` as build-dependency (add `[build-dependencies] toml = "0.8"`)
   - **No new runtime deps** needed for TOML DSL itself

4. **Binary size impact**:
   - 47 TOML files are plain text, ~200-500 bytes each = ~15KB total embedded
   - `toml` crate already linked (used by config.rs)
   - `build.rs` adds zero runtime cost
   - **Estimated impact: <100KB** - well within budget

5. **Upstream deps we skip** (telemetry-only):
   - `sha2` = telemetry hashing
   - `ureq` = HTTP client for telemetry
   - `hostname` = telemetry enrichment
   - `flate2` = telemetry compression
   - `quick-xml` = AWS XML parsing (needed if we port AWS module)

### P3.1 - TOML Part 1: Filter DSL engine + 14 built-in filters (#349)

- [ ] **P3.1a** - Core TOML filter engine (`src/toml_filter.rs`)
  - Upstream commit: `adda253`
  - Single file module (upstream uses `src/toml_filter.rs`, not a directory)
  - 8-stage pipeline:
    1. `strip_ansi` - remove ANSI escape codes
    2. `replace` - regex substitutions, line-by-line
    3. `match_output` - short-circuit on pattern match
    4. `strip/keep_lines` - filter lines by regex
    5. `truncate_lines_at` - truncate each line to N chars
    6. `head/tail_lines` - keep first/last N lines
    7. `max_lines` - absolute line cap
    8. `on_empty` - message if result is empty
  - Uses `lazy_static` for compiled regex (matches our pattern)
  - **Telemetry scrub**: remove any `telemetry::` calls

- [ ] **P3.1b** - build.rs for TOML concatenation
  - Copy upstream `build.rs`
  - Validates TOML at compile time, catches errors early
  - Detects duplicate filter names across files

- [ ] **P3.1c** - 14 built-in TOML filter files (`src/filters/`)
  - Copy from upstream: terraform-plan, make, ansible-playbook, helm, gcloud,
    iptables, fail2ban-client, sops, pre-commit, quarto, pio, shopify, trunk, mix
  - Each `.toml` file ~200-500 bytes

- [ ] **P3.1d** - Integration with main.rs routing
  - TOML-filtered commands route through engine
  - Fallback to raw command if TOML filter fails
  - Token tracking integration
  - Environment variables: `RTK_NO_TOML=1`, `RTK_TOML_DEBUG=1`

### P3.2 - TOML Part 2: User config + 4 more filters (#351)

- [ ] **P3.2a** - User-global TOML config
  - Upstream commit: `926e6a0`
  - Users can create custom TOML filters in `~/.config/rtk/filters.toml`
  - Project-local in `.rtk/filters.toml`
  - Shadow warning when user filter overrides built-in

- [ ] **P3.2b** - rtk init templates for TOML filters
  - Generate commented starter TOML filter template

- [ ] **P3.2c** - 4 additional built-in filters
  - tofu-plan, tofu-init, tofu-validate, tofu-fmt (OpenTofu variants)

### P3.3 - TOML Part 3: 15 more built-in filters (#386)

- [ ] **P3.3a** - 15 additional filter files
  - Upstream commit: `b71a8d2`
  - ping, rsync, dotnet-build, swift-build, shellcheck, hadolint, poetry-install,
    composer-install, brew-install, df, ps, systemctl-status, yamllint, markdownlint, uv-sync

### P3.4 - Rewrite engine improvements (from v0.29.0)

- [ ] **P3.4a** - Rewrite engine with shell_split + rewrite_segment
  - Upstream commit: `c1de10d` (#539)
  - Depends on TOML DSL being in place
  - `rtk rewrite` command uses TOML filter registry for routing
  - OpenCode plugin support (`rtk init -g --opencode`)
  - **Telemetry scrub**: verify no telemetry in rewrite paths

- [ ] **P3.4b** - Additional v0.29.0 items from develop merge (#499)
  - proxy quote-aware split with shell_split()
  - 11 new TOML filters (xcodebuild, jq, basedpyright, ty, skopeo,
    stat, biome, oxlint, jj, ssh, gcc)
  - Hook status detection and gain warnings
  - RTK_DISABLED overuse detection in discover

### P3 Quality Gates

After Phase 3:
- [ ] `cargo fmt --all && cargo clippy --all-targets && cargo test`
- [ ] **Binary size < 5MB**: `cargo build --release && ls -lh target/release/rtk`
  - If over budget: try `opt-level = "z"` or reduce embedded TOML count
- [ ] **Startup time < 10ms**: `hyperfine 'target/release/rtk git status' --warmup 3`
  - git status does NOT use TOML filters, so should be unaffected
  - Also test: `hyperfine 'target/release/rtk terraform plan' --warmup 3` (TOML path)
- [ ] **TOML filter test**: `RTK_TOML_DEBUG=1 rtk make` to verify filter matching
- [ ] All inline TOML tests pass (filters have `[[tests.<name>]]` sections)
- [ ] Update CLAUDE.md, ARCHITECTURE.md with TOML filter architecture
- [ ] Update README.md with supported commands list

### P3 Escape Hatch

If Phase 3 causes problems (binary too large, startup regression, test failures):
1. Revert the sync/upstream-p3-toml-dsl branch
2. Release v0.26.0 with Phase 1 + Phase 2 only
3. Revisit TOML DSL with a minimal port: engine + 5 highest-value filters only
   (terraform, make, helm, gcloud, shellcheck)

---

## Explicitly Skipped

| Upstream Feature | Reason |
|-----------------|--------|
| Anonymous telemetry (#334) | Privacy - no phone-home behavior |
| Telemetry enrichment (#462, #469, #471) | Depends on skipped telemetry |
| `sha2`, `ureq`, `hostname`, `flate2` deps | Telemetry-only dependencies |
| Claude Code skills for PR triage (#343) | Repo-specific CI tooling |
| README translations (zh, ko, ja, fr, es) | Cosmetic |
| CI Discord notifications (#375) | Infra-specific |
| DCO check workflow | Governance-specific |
| CONTRIBUTING.md | Docs-only |
| install-local.sh self-contained (#89) | We use Homebrew tap |

### Skipped but verify

| Feature | We have | Action |
|---------|---------|--------|
| SHA-256 hook integrity (#119) | `src/hook_audit_cmd.rs` exists | Verify our hook_audit is independent |
| Hook audit mode (#151) | `hooks/` directory exists | Verify our hooks work without upstream's system |
| Outdated hook warning (#344/#350) | N/A | Skip - our hooks are self-managed |

---

## Documentation Updates (per phase)

### After Phase 1
- [ ] No doc changes needed (bug fixes only)

### After Phase 2
- [ ] CLAUDE.md: Add aws, psql, gt modules to Module Responsibilities table
- [ ] ARCHITECTURE.md: Update module count, add new modules to table
- [ ] README.md: Add usage examples for new commands

### After Phase 3
- [ ] CLAUDE.md: Add TOML filter DSL to Architecture section
- [ ] ARCHITECTURE.md: Add TOML Filter DSL architecture section
- [ ] README.md: Update supported commands list with TOML-filtered commands

---

## Execution Rules

1. Work through items sequentially within each phase
2. Use reference implementation approach - read upstream diff, manually adapt
3. Run `cargo fmt --all && cargo clippy --all-targets && cargo test` after each change
4. For new modules: copy upstream file, adapt imports, scrub telemetry, add tests
5. Mark items done with `[x]` as we complete them
6. Commit messages: `fix(sync):` for bug fixes, `feat(sync):` for features
7. One branch per phase, merge to master after phase quality gates pass

## Reference

- Upstream repo: https://github.com/rtk-ai/rtk
- Upstream at tag: `git show v0.29.0:<path>` or `git show upstream/master:<path>`
- View upstream file: `git show upstream/master:src/<file>.rs`
- View upstream diff for commit: `git show <sha> -- <file>`
- Compare files: `git diff HEAD upstream/master -- src/<file>.rs`
