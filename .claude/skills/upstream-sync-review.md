---
name: upstream-sync-review
description: >
  Weekly review of rtk-ai/rtk upstream for new fixes and features worth porting.
  Run this skill to get a prioritized list of improvements since the last sync.
  Produces a P1/P2/P3 backlog and saves a plan to docs/plans/.
---

# Upstream Sync Review

Use this skill when the user asks to review the upstream repository for new changes, or on the weekly cadence. Output is a prioritized diff of what upstream shipped vs. what we have.

## Trigger

User says: "review upstream", "check upstream", "sync review", "what's new upstream", or runs `/upstream-sync-review`.

## Step 1: Establish last sync point

Read memory to find the last synced upstream tag:

```bash
cat /Users/mk/.claude/projects/-Users-mk-Code-rtk/memory/project_upstream_sync.md
```

The memory file contains a line like `**Last synced upstream tag:** v0.36.0`. Extract that tag. If absent, default to `v0.34.0` and note the gap.

Also check the current branch to understand in-progress sync work:

```bash
git branch --show-current
git log --oneline master..HEAD | head -10
```

## Step 2: Fetch upstream releases since last sync

```bash
gh release list --repo rtk-ai/rtk --limit 20
```

Identify all stable releases (exclude pre-release `dev-*` tags) published after the last synced tag. If none, report "upstream is in sync" and stop.

For each new stable release (e.g. `v0.37.0`), fetch the changelog:

```bash
gh release view v0.37.0 --repo rtk-ai/rtk --json body --jq '.body'
```

## Step 3: Get full commit list between last sync and current upstream

```bash
gh api "repos/rtk-ai/rtk/compare/LAST_TAG...LATEST_TAG" \
  --jq '.commits[] | "\(.sha[0:8]) \(.commit.message | split("\n")[0])"'
```

If the commit list exceeds 100 entries, limit to release changelogs only.

## Step 4: Cross-reference against our codebase

For each upstream change, determine applicability. Check which source modules we have:

```bash
ls src/*.rs src/**/*.rs 2>/dev/null | sed 's|src/||; s|\.rs||'
```

**Always skip** (we don't want these):
- `telemetry` / `RGPD` / privacy / consent changes
- Multipass VM integration test suite
- Swift ecosystem tests
- Ruby, .NET ecosystem filters (unless we have them)
- Personal preference changes in upstream CLAUDE.md
- Upstream docs pointing to their own website

**Always include** if upstream has it and we have the corresponding module:
- Bug fixes in modules we have (git, cargo, go, pnpm, grep, gh, curl, ls, find, pytest, etc.)
- Exit code correctness fixes
- Panic/crash fixes
- Security fixes in hook/permission system
- New TOML filters (we have the TOML engine)

**Include if impactful**:
- New command modules (assess by token savings %)
- CLI flag fixes that affect daily use (git push -u, git diff --)

## Step 5: Classify by priority

Assign priority based on user impact:

**P1 - Critical** (ship immediately):
- Panics / crashes
- Correctness: wrong output, silently broken commands, exit codes
- Security: hook permission bypasses
- User-visible regressions from upstream patches

**P2 - Important** (next batch):
- Filter quality improvements (missing output, truncation issues)
- False positives/negatives in filter output
- Missing flag support that breaks common workflows
- Cross-platform fixes (Windows, Linux temp dir, etc.)

**P3 - Enhancement** (future batch):
- New command modules with ≥60% token savings
- New TOML filters for tools we use
- UX improvements (better summaries, timezone display, etc.)

## Step 6: Check for items already ported

For each P1/P2 item, verify we haven't already ported it:

```bash
git log --oneline --all | grep -i "upstream.*#NUMBER\|#NUMBER"
```

If the item is already in our git history (referenced by upstream issue/PR number), mark it as "done" and exclude it.

## Step 7: Produce the report

Format the output as:

```
## Upstream Sync Review: LAST_TAG -> LATEST_TAG
Date: YYYY-MM-DD
New releases: vX.Y.Z, vA.B.C

### P1 - Critical (N items)
| # | Upstream ref | What it fixes | Our module | Status |
|---|---|---|---|---|
| 1 | #1234 | description | src/foo.rs | pending |

### P2 - Filter quality (N items)
(same table)

### P3 - New features (N items)
(same table)

### Already ported (N items)
(brief list)

### Skipped (N items, reason)
(brief list)

**Recommendation:** [1-2 sentences on whether to create a plan now or wait]
```

## Step 8: Save as a plan (if user confirms)

If the user says "yes, create the plan" or "implement this", use **superpowers:writing-plans** to write a full task-by-task implementation plan to:

```
docs/plans/YYYY-MM-DD-upstream-sync-vN.md
```

Where N is the sync version number (increment from the last sync plan found in `docs/plans/`).

## Step 9: Update memory

After the review (not after implementation), update the sync memory file to record what was reviewed and when:

In `/Users/mk/.claude/projects/-Users-mk-Code-rtk/memory/project_upstream_sync.md`, update:
- `**Last reviewed:** YYYY-MM-DD` - today's date
- `**Last synced upstream tag:** vX.Y.Z` - only update this after implementation is merged to master

Do NOT update "last synced tag" based on the review alone - only update it when the sync branch is merged.

## Cadence guidance

Run this review weekly (Monday is a good default). Upstream ships roughly 1-2 stable releases per week. A batch of P1 items should be ported within 3 days of appearing upstream. P2/P3 can wait for the next planned sync.

## Anti-patterns to avoid

- **Don't go deep into telemetry PRs** - we skip all of those, don't waste time reading them
- **Don't port large architectural refactors** - upstream's `refacto-folders-and-documentation` and unified exit-code flow rewrites are not worth the merge effort given our diverged codebase
- **Don't blindly copy function signatures** - upstream may have added parameters we don't have; adapt to our existing function shapes
- **Don't skip the cross-reference step** - half the items you find won't apply to our modules or will already be ported
