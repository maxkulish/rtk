# Upstream Sync Plan v2: v0.29.0 -> v0.34.0

**Created**: 2026-03-28
**Our fork**: maxkulish/rtk v0.25.1 (synced through upstream v0.29.0)
**Upstream**: rtk-ai/rtk v0.34.0 (2026-03-26)
**Strategy**: Manual porting by risk level - bug fixes first, then features
**Previous sync**: docs/2026-03-14-upstream-sync-plan.md (completed P1-P3)

---

## Pre-Flight Checklist

- [ ] Verify upstream remote: `git remote get-url upstream` or add with `git remote add upstream https://github.com/rtk-ai/rtk.git`
- [ ] Fetch upstream: `git fetch upstream --tags`
- [ ] Baseline test count: `cargo test 2>&1 | tail -1`
- [ ] Baseline binary size: `ls -lh target/release/rtk`
- [ ] Baseline startup time: `hyperfine 'target/release/rtk git status' --warmup 3`

---

## Branch Strategy

```
master (v0.25.1)
  |
  +-- sync/v2-p1-correctness
  |     |-- git log chain: #505 -> #546 -> #619 -> #833
  |     |-- diff/read fixes: #824 (3 commits)
  |     |-- critical bugs: #626
  |     |-- cargo diagnostics: #738
  |     +-- merge to master, release patch
  |
  +-- sync/v2-p2-cli-parity
  |     |-- passthrough fallback: #200
  |     |-- gh fixes: #775, #773, #196
  |     |-- golangci-lint v2: #722
  |     +-- merge to master, release minor
  |
  +-- sync/v2-p3-features
        |-- rtk wc: #175
        |-- rtk gain -p: #128
        |-- TOML filters + rewrite rules
        |-- rtk session: #547
        +-- merge to master, release minor
```

---

## Conflict Resolution Protocol

Same as previous sync - see docs/2026-03-14-upstream-sync-plan.md.

Key points:
1. Read upstream diff first, adapt to our architecture
2. Preserve fork-specific additions (GitGlobalOpts, ensure_failure_visibility, etc.)
3. Scrub telemetry calls from ported code
4. Run `cargo fmt --all && cargo clippy --all-targets && cargo test` after each change

---

## Telemetry Scrubbing

Same policy as previous sync - do NOT port telemetry. See previous plan for details.

---

## Phase 1 - Output Correctness (HIGH priority, LOW risk)

Things that produce wrong, missing, or truncated output. Isolated filter fixes.

### Dependency Order for git.rs changes

These MUST be ported in strict order due to overlapping code:

```
#505 (handle -n N / --max-count=N)
  -> #546 (preserve commit body)
    -> #619 (--oneline regression fix)
      -> #833 (exact truncation counts)
```

### P1.1 - Git log limit parsing (#505) -- ALREADY DONE

- **Status**: Already implemented in previous sync. `parse_user_limit()` at `src/git.rs:370` handles `-N`, `--max-count=N`, and `-n N` forms.
- **No work needed.**

### P1.2 - Preserve commit body in git log (#546)

- **Upstream PR**: #546 - "fix: preserve commit body in git log output"
- **Upstream commit**: `c3416eb`
- **Files**: `src/git.rs`, `src/cargo_cmd.rs`
- **Stats**: +70/-22
- **Problem**: Git log output loses first line of commit body (important for multi-line commit messages)
- **Fix**: Add `%b` (body) to git log format, extract first meaningful body line
- **Approach**: Reference impl - check format string and body extraction logic
- **Depends on**: P1.1

### P1.3 - Git log --oneline regression (#619)

- **Upstream PR**: #619 - "fix: git log --oneline regression drops commits"
- **Upstream commit**: `8e85d67`
- **Files**: `src/git.rs`
- **Stats**: +96/-18
- **Problem**: After #546, `--oneline` mode broke because it splits on `---END---` markers that only exist with RTK's custom format
- **Fix**: Use line-based truncation when user specifies own format (--oneline, --pretty, --format)
- **Approach**: Reference impl - check how format detection triggers different truncation paths
- **Depends on**: P1.2

### P1.4 - Exact truncation counts (#833)

- **Upstream PR**: #833 - "fix(truncation): accurate overflow counts and omission indicators"
- **Upstream commit**: `185fb97`
- **Files**: `src/diff_cmd.rs`, `src/filter.rs`, `src/git.rs`, `src/grep_cmd.rs`
- **Stats**: +263/-48
- **Problem**: Vague "(N more...)" truncation markers give inaccurate counts
- **Fix**: 3 sub-fixes:
  1. `condense_unified_diff` overflow count in diff_cmd.rs
  2. `compact_diff` hunk truncation message in git.rs
  3. `filter_log_output` silently dropped body lines in git.rs
- **Approach**: Reference impl - touch multiple files but changes are localized
- **Depends on**: P1.3

### P1.5 - Never truncate diff content (#827)

- **Upstream issue**: #827 - "silent truncation causes data loss - LLM takes decisions on incomplete diffs"
- **Upstream commit**: `80fc29a` (in PR #824)
- **Files**: `src/diff_cmd.rs`
- **Problem**: `rtk git diff` truncated diff output, causing LLMs to make decisions on incomplete information
- **Fix**: Remove truncation limits from diff content display
- **Approach**: Check our diff_cmd.rs, apply equivalent change
- **Independent of**: git.rs chain above

### P1.6 - rtk read defaults to no filtering (#822)

- **Upstream issue**: #822
- **Upstream commit**: `5e0f3ba` (in PR #824)
- **Files**: `src/main.rs`, `src/read.rs`
- **Problem**: `rtk read` applied code filtering by default, stripping comments and whitespace from files the LLM needs to see in full
- **Fix**: Default filter level to `none` instead of `minimal`
- **Approach**: Check our read command routing in main.rs and read.rs

### P1.7 - Binary file detection (#822)

- **Upstream commit**: `8886c14` (in PR #824)
- **Files**: `src/read.rs`
- **Problem**: Running `rtk read` on binary files produces empty output when filter fails
- **Fix**: Detect binary files and skip filtering, or return error message
- **Approach**: Port binary detection logic to our read.rs

### P1.8 - 6 critical bugs: exit codes, unwrap, lazy regex (#626)

- **Upstream PR**: #626 - "fix: 6 critical bugs"
- **Upstream commit**: `3005ebd`
- **Files**: `src/container.rs`, `src/golangci_cmd.rs`, `src/log_cmd.rs`, `src/prisma_cmd.rs`
- **Stats**: +91/-46
- **The 6 bugs**:
  1. `container.rs`: docker ps/images ignored exit code
  2. `container.rs`: `items.is_none() || items.unwrap()` double-unwrap pattern (safety)
  3. `golangci_cmd.rs`: all exit codes swallowed
  4. `log_cmd.rs`: 5x `Regex::new().unwrap()` compiled per call -> `lazy_static!`
  5. `prisma_cmd.rs`: 3x `anyhow::bail!` hid actual error output
  6. `mypy_cmd.rs`: exit code - **SKIP** (we don't have this module as standalone)
- **Approach**: Port 5 of 6 fixes (skip mypy standalone module)
- **Conflict note**: golangci_cmd.rs also touched by P2.5, port this first

### P1.9 - Preserve cargo test compile diagnostics (#738)

- **Upstream PR**: #738 - "fix(cargo): preserve test compile diagnostics"
- **Upstream commit**: `15d5beb`
- **Files**: `src/cargo_cmd.rs`
- **Stats**: +39/-0
- **Problem**: When `cargo test` has compile errors, RTK filters them out - user sees nothing
- **Fix**: Detect and preserve compiler diagnostic output before applying test filter
- **Approach**: Additive change, low conflict risk

### P1 Quality Gates

- [ ] `cargo fmt --all && cargo clippy --all-targets && cargo test`
- [ ] Binary size still < 5MB
- [ ] Startup time still < 10ms
- [ ] Manual tests:
  - [ ] `rtk git log --oneline -5` - shows exactly 5 commits, none dropped
  - [ ] `rtk git log -n 3` - shows exactly 3 commits
  - [ ] `rtk git diff` on a file with 100+ changed lines - nothing truncated
  - [ ] `rtk read Cargo.toml` - shows full file, no filtering
  - [ ] `rtk read` on a binary file - no empty output
  - [ ] `rtk cargo test` with a compile error - diagnostic visible

---

## Phase 2 - CLI Flag Parity (HIGH priority, MEDIUM risk)

Flags and commands that silently break or error instead of passing through.

### P2.1 - Passthrough fallback on Clap parse failure (#200) -- ALREADY DONE

- **Status**: Already implemented. `should_fallback()` at `src/main.rs:1802` handles `InvalidSubcommand`, `UnknownArgument`, `InvalidValue`, `NoEquals` errors. Falls back via `run_fallback()`.
- **No work needed.**

### P2.2 - gh: passthrough --comments flag (#720, #775)

- **Upstream PR**: #775 - "fix(gh): passthrough --comments flag in issue/pr view"
- **Upstream commit**: `75cd223`
- **Files**: `src/gh_cmd.rs`
- **Stats**: +49/-3
- **Problem**: `rtk gh issue view 123 --comments` silently drops the --comments flag
- **Fix**: Detect `--comments` and pass through to raw gh command
- **Approach**: Small, isolated change to gh_cmd.rs
- **Test**: `rtk gh issue view 1 --comments` should show comments

### P2.3 - gh: skip compact_diff for --name-only/--stat (#730, #773)

- **Upstream PR**: #773 - "fix(gh): skip compact_diff for --name-only/--stat"
- **Upstream commit**: `c576249`
- **Files**: `src/gh_cmd.rs`
- **Stats**: +55/-1
- **Problem**: `rtk gh pr diff 123 --name-only` returns empty output because compact_diff filters out file-only output
- **Fix**: Detect `--name-only` and `--stat` flags, skip compact_diff
- **Approach**: Small, isolated change
- **Test**: `rtk gh pr diff 1 --name-only` should list changed files

### P2.4 - gh: skip rewrite for --json/--jq/--template (#196)

- **Upstream**: Landed via PR #499 (develop->master merge), commit `079ee9a`
- **Files**: `src/gh_cmd.rs`, `src/discover/registry.rs`
- **Problem**: `rtk gh pr view 123 --json title` gets filtered when it should pass through raw JSON
- **Fix**: Add `has_json_flag()` check to skip RTK filtering when user requests structured output
- **Approach**: MEDIUM risk - the upstream change is tangled in a 43-file mega-merge. Extract manually.
- **Test**: `rtk gh pr view 1 --json title,state` should output raw JSON

### P2.5 - golangci-lint v2 compatibility (#722)

- **Upstream PR**: #722 - "fix(golangci-lint): add v2 compatibility with runtime version detection"
- **Upstream commit**: `3480ce5`
- **Files**: `src/golangci_cmd.rs`, `tests/fixtures/golangci_v2_json.txt`
- **Stats**: +424/-24
- **Problem**: golangci-lint v2 changed JSON output format, breaking RTK's parser
- **Fix**: Runtime version detection (`golangci-lint version`), parse v1 or v2 JSON accordingly
- **Approach**: Reference impl. Port after P1.8 (which fixes exit codes in same file).
- **Test**: Run with golangci-lint v2 output fixture

### P2 Quality Gates

- [ ] `cargo fmt --all && cargo clippy --all-targets && cargo test`
- [ ] Binary size still < 5MB
- [ ] Startup time still < 10ms
- [ ] Manual tests:
  - [ ] `rtk git worktree list` - works (passthrough fallback)
  - [ ] `rtk gh issue view 1 --comments` - shows comments
  - [ ] `rtk gh pr diff 1 --name-only` - lists files
  - [ ] `rtk gh pr view 1 --json title` - raw JSON output
  - [ ] `rtk golangci-lint run ./...` - works with v2

---

## Phase 3 - New Commands & Features (MEDIUM priority)

New functionality. Each item is largely independent.

### P3.1 - rtk wc command (#175)

- **Upstream PR**: #175 - "feat: add rtk wc command"
- **Upstream commit**: `393fa5b`
- **Files**: `src/wc_cmd.rs` (new), `src/main.rs`
- **Stats**: +413/-0
- **What**: Compact word/line/byte counts output
- **Approach**: Clean new module, copy from upstream, register in main.rs
- **Risk**: LOW

### P3.2 - rtk gain -p per-project savings (#128)

- **Upstream PR**: #128 - "feat(gain): add per-project token savings with -p flag"
- **Upstream commit**: `2b550ee`
- **Files**: `src/gain.rs`, `src/main.rs`, `src/tracking.rs`
- **Stats**: +275/-51
- **What**: Filter token savings by project directory
- **Approach**: Reference impl - touches tracking.rs schema, may need migration
- **Risk**: MEDIUM (schema change)

### P3.3 - 11 new TOML filters

- **Upstream**: Various PRs from v0.29.0+
- **New filters**: xcodebuild, jq, basedpyright, ty, skopeo, stat, biome, oxlint, jj, ssh, gcc
- **What**: Additional TOML filter definitions for our existing engine
- **Approach**: Copy .toml files to src/filters/, rebuild
- **Risk**: LOW (just data files, engine already ported)

### P3.4 - TOML-filtered commands in hook rewrite rules (#475)

- **Upstream PR**: #475
- **Upstream commit**: `91289d7`
- **Files**: `src/discover/registry.rs`, `src/discover/rules.rs`, `src/rewrite_cmd.rs`
- **Stats**: +300/-13
- **What**: 32 TOML-filtered commands added to hook rewrite rules so Claude Code hooks route them through RTK
- **Approach**: Reference impl - adapt to our discover/rewrite infrastructure
- **Risk**: MEDIUM (touches rewrite routing)

### P3.5 - rtk session command (#547)

- **Upstream PR**: #547 - "feat: add rtk session command"
- **Upstream commit**: `be67d66`
- **Files**: `src/session_cmd.rs` (new), `src/main.rs`, `src/discover/`
- **Stats**: +491/-1
- **What**: Adoption overview by scanning Claude Code JSONL session logs
- **Approach**: Copy module, adapt to our discover infrastructure
- **Risk**: MEDIUM (depends on discover infra)

### P3 Quality Gates

- [ ] `cargo fmt --all && cargo clippy --all-targets && cargo test`
- [ ] Binary size still < 5MB
- [ ] Startup time still < 10ms
- [ ] Manual tests:
  - [ ] `rtk wc README.md` - shows compact counts
  - [ ] `rtk gain -p` - shows per-project savings
  - [ ] `RTK_TOML_DEBUG=1 rtk jq '.' file.json` - TOML filter matches
  - [ ] `rtk session` - shows session overview

---

## Explicitly Skipped

| Upstream Feature | Reason |
|-----------------|--------|
| Multi-agent init (Cursor, Windsurf, Copilot, Gemini, Cline, OpenCode) | Fork uses Claude Code only |
| Anonymous telemetry (#334, #471, #640) | No phone-home in fork |
| Ruby on Rails (rspec, rubocop, rake, bundle) | Not needed currently |
| Swift / .NET structured support | Not needed currently |
| Hook permission deny/ask rules | Complex, tied to upstream rewrite architecture |
| License change to Apache 2.0 | Separate decision |
| Claude Code skills/agents/rules | Repo-specific CI tooling |
| Worktree slash commands | Claude Code specific |
| README translations | Cosmetic |
| Discord notifications | Infra-specific |
| Pre-release tag system | Not needed for fork |
| Trust boundary for project-local TOML (#623) | Revisit if users request custom filters |

---

## Conflict Risk Map

| File | Touched by | Risk | Strategy |
|------|-----------|------|----------|
| `src/git.rs` | P1.1, P1.2, P1.3, P1.4 | HIGH | Strict sequential order, reference impl |
| `src/main.rs` | P1.6, P2.1, P3.1, P3.2, P3.5 | HIGH | Adapt each to our routing |
| `src/gh_cmd.rs` | P2.2, P2.3, P2.4 | MEDIUM | Independent changes, same file |
| `src/golangci_cmd.rs` | P1.8, P2.5 | MEDIUM | Port P1.8 first, then P2.5 |
| `src/diff_cmd.rs` | P1.4, P1.5 | LOW | Different functions |
| `src/cargo_cmd.rs` | P1.2, P1.9 | LOW | Different functions |
| `src/read.rs` | P1.6, P1.7 | LOW | Self-contained |
| `src/tracking.rs` | P2.1, P3.2 | MEDIUM | Schema changes |

---

## Execution Rules

1. Work through phases sequentially (P1 -> P2 -> P3)
2. Within P1, the git.rs chain MUST be in order: P1.1 -> P1.2 -> P1.3 -> P1.4
3. Other P1 items (P1.5-P1.9) can be done in any order
4. Use reference implementation approach - read upstream diff, adapt to our code
5. Run `cargo fmt --all && cargo clippy --all-targets && cargo test` after each change
6. Commit messages: `fix(sync):` for bug fixes, `feat(sync):` for features
7. One branch per phase, merge to master after quality gates pass
8. Scrub telemetry from all ported code

## Reference

- Upstream repo: https://github.com/rtk-ai/rtk
- View upstream file: `git show upstream/master:src/<file>.rs`
- View upstream PR diff: `gh api repos/rtk-ai/rtk/pulls/<N>/files --jq '.[].filename'`
- View upstream commit: `gh api repos/rtk-ai/rtk/commits/<sha> --jq '.files[].filename'`
- Previous sync plan: docs/2026-03-14-upstream-sync-plan.md
