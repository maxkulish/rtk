# rtk - Rust Token Killer

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

**High-performance CLI proxy to minimize LLM token consumption.**

[Upstream](https://github.com/rtk-ai/rtk) | [GitHub](https://github.com/maxkulish/rtk) | [Website](https://www.rtk-ai.app)

rtk sits between your LLM and the shell, filtering and compressing command outputs before they reach the context window. It saves 60-90% of tokens on common development operations through smart filtering, grouping, truncation, and deduplication.

## Token Savings (30-min Claude Code Session)

Typical session without rtk: **~150,000 tokens**
With rtk: **~45,000 tokens** - **70% reduction**

| Operation | Frequency | Standard | rtk | Savings |
|-----------|-----------|----------|-----|---------|
| `ls` / `tree` | 10x | 2,000 | 400 | -80% |
| `cat` / `read` | 20x | 40,000 | 12,000 | -70% |
| `grep` / `rg` | 8x | 16,000 | 3,200 | -80% |
| `git status` | 10x | 3,000 | 600 | -80% |
| `git diff` | 5x | 10,000 | 2,500 | -75% |
| `git log` | 5x | 2,500 | 500 | -80% |
| `git add/commit/push` | 8x | 1,600 | 120 | -92% |
| `npm test` / `cargo test` | 5x | 25,000 | 2,500 | -90% |
| `ruff check` | 3x | 3,000 | 600 | -80% |
| `pytest` | 4x | 8,000 | 800 | -90% |
| `go test` | 3x | 6,000 | 600 | -90% |
| `docker ps` | 3x | 900 | 180 | -80% |
| **Total** | | **~118,000** | **~23,900** | **-80%** |

> Estimates based on medium-sized TypeScript/Rust projects. Actual savings vary by project size.

## Install

```bash
brew install maxkulish/tap/rtk
```

> Note: Use `maxkulish/tap/rtk`, not Homebrew core's `rtk` (different project).

Verify:
```bash
rtk --version  # Should show "rtk 0.25.0" <!-- x-release-please-version -->
rtk gain       # Should show token savings stats
```

## Activate

```bash
rtk init --global
# Follow printed instructions to patch ~/.claude/settings.json
# Restart Claude Code

# Test it works:
git status  # Should show compact output via rtk
```

## Commands

Global flags: `-u` / `--ultra-compact` (extra savings), `-v` / `--verbose` (increase verbosity)

### Files
```bash
rtk ls .                        # Token-optimized directory tree
rtk read file.rs                # Smart file reading
rtk read file.rs -l aggressive  # Signatures only (strips bodies)
rtk smart file.rs               # 2-line heuristic code summary
rtk find "*.rs" .               # Compact find results
rtk grep "pattern" .            # Grouped search results
```

### Git
```bash
rtk git status                  # Compact status
rtk git log -n 10               # One-line commits
rtk git diff                    # Condensed diff
rtk git add .                   # -> "ok"
rtk git commit -m "msg"         # -> "ok abc1234"
rtk git push                    # -> "ok main"
rtk git pull                    # -> "ok 3 files +10 -2"
```

### Tests & Build
```bash
rtk test cargo test             # Show failures only (-90%)
rtk err npm run build           # Errors/warnings only
rtk vitest run                  # Vitest failures only (-99.5%)
rtk pytest                      # Pytest failures only (-90%)
rtk go test                     # Go test NDJSON parser (-90%)
rtk go build                    # Build errors only (-80%)
rtk go vet                      # Vet issues (-75%)
```

### Linting & Formatting
```bash
rtk lint                        # ESLint/Biome grouped by rule (-84%)
rtk tsc                         # TypeScript errors grouped by file (-83%)
rtk ruff check                  # Ruff linter JSON output (-80%)
rtk ruff format                 # Ruff formatter text filter
rtk golangci-lint run           # Go linter grouped by rule (-85%)
rtk prettier --check .          # Files needing formatting (-70%)
```

### Packages
```bash
rtk pnpm list                   # Compact dependency tree (-70%)
rtk pnpm outdated               # Available updates (-90%)
rtk pnpm install pkg            # Silent installation
rtk pip list                    # Package list, auto-detect uv (-70%)
rtk pip install pkg             # Install with compact output
rtk pip outdated                # Outdated packages (-85%)
```

### Data & Analytics
```bash
rtk json config.json            # Structure without values
rtk deps                        # Dependencies summary
rtk env -f AWS                  # Filtered env vars
rtk gain                        # Token savings summary
rtk gain --graph                # With ASCII graph (last 30 days)
rtk gain --history              # With recent command history
rtk gain --daily                # Day-by-day breakdown
rtk gain --all --format json    # JSON export for dashboards
rtk discover                    # Find missed savings opportunities
rtk discover --all              # Across all Claude Code projects
```

### Containers
```bash
rtk docker ps                   # Compact container list
rtk docker images               # Compact image list
rtk docker logs <container>     # Deduplicated logs
rtk kubectl pods                # Compact pod list
rtk kubectl logs <pod>          # Deduplicated logs
rtk kubectl services            # Compact service list
```

### GitHub CLI
```bash
rtk gh pr list                  # Compact PR listing
rtk gh pr view 42               # PR details + checks summary
rtk gh issue list               # Compact issue listing
rtk gh run list                 # Workflow run status
```

### Other
```bash
rtk log app.log                 # Deduplicated logs with counts
rtk summary <long command>      # Heuristic summary
rtk wget https://example.com   # Download, strip progress bars
rtk proxy <any command>         # Passthrough with usage tracking
rtk config                      # Show config (--create to generate)
```

## Examples

**Directory listing** - `ls -la` (45 lines, ~800 tokens) vs `rtk ls` (12 lines, ~150 tokens):
```
my-project/
  src/ (8 files)
    main.rs
    lib.rs
  Cargo.toml
  README.md
```

**Git push** - `git push` (15 lines, ~200 tokens) vs `rtk git push` (1 line, ~10 tokens):
```
ok main
```

**Tests** - `cargo test` (200+ lines on failure) vs `rtk test cargo test` (~20 lines):
```
FAILED: 2/15 tests
  test_edge_case: assertion failed at src/lib.rs:42
  test_overflow: panic at src/utils.rs:18
```

## Documentation

- [TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md) - Fix common issues
- [ARCHITECTURE.md](ARCHITECTURE.md) - Technical architecture and development guide
- [SECURITY.md](SECURITY.md) - Security policy and vulnerability reporting
- [Tracking API](docs/tracking.md) - Programmatic access to token savings data
- [Audit Guide](docs/AUDIT_GUIDE.md) - Token savings analytics and data export

Uninstall: `brew uninstall maxkulish/tap/rtk` and `rtk init -g --uninstall`

## Contributing

Contributions welcome! Please open an issue or PR on [GitHub](https://github.com/maxkulish/rtk/issues).

## License

MIT License - see [LICENSE](LICENSE) for details.

## Contact

- Website: https://www.rtk-ai.app
- Email: contact@rtk-ai.app
