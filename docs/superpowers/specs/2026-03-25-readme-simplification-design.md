# Spec: Simplify README.md and Delete INSTALL.md

**Date**: 2026-03-25
**Status**: Draft

## Problem

README.md (860 lines) and INSTALL.md (376 lines) have massive overlap, multiple installation methods (Homebrew, curl, cargo, binaries), hook deep-dives, maintainer docs, and troubleshooting - far more than a user needs to get started. The canonical install path is Homebrew via `maxkulish/tap/rtk`.

## Decision

1. **Homebrew-only installation** - remove curl script, cargo from git, cargo from upstream, and pre-built binary methods
2. **Delete INSTALL.md** - with Brew-only, 5 lines of install instructions don't justify a separate file
3. **Slim README to ~120-150 lines** - install, activate, command overview, token savings, links to docs/

## New README.md Structure

### 1. Header + Tagline (~5 lines)
- Project name, MIT badge, one-line description
- Links: upstream repo, this fork, website

### 2. What It Does (~15 lines)
- One paragraph: proxy concept, 60-90% token savings
- Token savings table (the existing 30-min session table, kept as-is - it's the strongest selling point)

### 3. Install (~10 lines)
```
brew install maxkulish/tap/rtk
```
- One-line note: "Use `maxkulish/tap/rtk`, not Homebrew core's `rtk` (different project)"
- Verify with version line carrying the release-please annotation:
  `rtk --version  # Should show "rtk 0.25.0" <!-- x-release-please-version -->`
- Verify: `rtk gain`

### 4. Activate (~10 lines)
```
rtk init --global
```
- Follow printed instructions (patch settings.json)
- Restart Claude Code
- Test: run `git status` - should show compact output

### 5. Commands (~50-60 lines)
Single code block per group, one command per line, no blank lines between entries:
- Files: ls, read, smart, find, grep
- Git: status, log, diff, add, commit, push, pull
- Tests & Build: cargo test, vitest, pytest, go test, err
- Linting: lint, tsc, ruff, golangci-lint, prettier
- Packages: pnpm, pip
- Data: json, deps, env, gain, discover
- Containers: docker, kubectl
- GitHub: gh pr, gh issue, gh run
- Other: log, summary, wget, proxy

Global flags (`-u`/`--ultra-compact`, `-v`/`--verbose`) noted as a one-liner before command groups.

### 6. Examples (~15 lines)
Keep the 3 before/after comparisons (ls, git push, cargo test) - they're effective and concise.

### 7. Footer (~10 lines)
- Documentation links: TROUBLESHOOTING.md, ARCHITECTURE.md, SECURITY.md, tracking.md, AUDIT_GUIDE.md
- Uninstall: `brew uninstall maxkulish/tap/rtk` + `rtk init -g --uninstall`
- Contributing, license, contact

## Content Removed from README

| Section | Disposition |
|---------|------------|
| Name collision warning (13 lines) | Replaced with 1-line note in Install section |
| Pre-installation check block | Removed (Brew tap is unambiguous) |
| curl install script | Removed |
| cargo install methods | Removed |
| Pre-built binaries | Removed |
| Installation modes table | Removed (only one mode: `rtk init -g`) |
| Installation flags detail | Removed |
| Custom database path | Already in CLAUDE.md, not needed in README |
| Tee output recovery | Already in CLAUDE.md, not needed in README |
| How It Works diagram + four strategies (lines 336-367) | Removed (lives in ARCHITECTURE.md) |
| Hook "What Are Hooks?" explainer | Removed |
| Hook architecture diagram | Removed |
| Hook manual install steps | Removed |
| Per-project install | Removed |
| Commands rewritten table | Removed (hook is transparent) |
| Suggest hook section | Removed |
| Discover detailed explanation + example output (lines 207-246) | Reduced to single line in commands section |
| Configuration gain session example (lines 417-441) | Removed (covered in docs/AUDIT_GUIDE.md) |
| Uninstalling detailed section (lines 689-720) | Reduced to 2 lines in footer |
| Troubleshooting (settings.json, hook, uninstall) | Link to docs/TROUBLESHOOTING.md |
| For Maintainers / Security review (lines 795-843) | Removed, link to SECURITY.md |
| Contributing section | Kept as 1-line in footer |
| License section | Kept as 1-line in footer |
| Contact section | Kept as 1-line in footer |

## Files Changed

| File | Action |
|------|--------|
| README.md | Rewrite to ~120-150 lines |
| INSTALL.md | Delete |
| README.md line 7 | Remove `[Install](INSTALL.md)` link from header |
| README.md line 725 | Remove `[INSTALL.md](INSTALL.md)` from docs list |
| install.sh | Keep as undocumented fallback (not referenced from README) |

Note: No other files in the repo reference INSTALL.md (confirmed via grep). CLAUDE.md and docs/ do not link to it.

## Constraints

- README.md must keep `<!-- x-release-please-version -->` annotation on the version line (in Install/verify section)
- `install.sh` script is kept but no longer documented - serves as undocumented fallback
- No content is truly lost - detailed architecture, hook internals, config, and discover details live in CLAUDE.md, ARCHITECTURE.md, docs/AUDIT_GUIDE.md, and docs/

## Success Criteria

- New user can install and activate rtk in under 2 minutes by reading only the README
- README is ~120-150 lines
- INSTALL.md no longer exists
- No broken links in remaining docs
- `<!-- x-release-please-version -->` annotation present and functional
