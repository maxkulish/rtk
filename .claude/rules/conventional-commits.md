# Conventional Commits

RTK uses [Conventional Commits](https://www.conventionalcommits.org/) to drive automated releases via Release Please. Every commit to master must follow this format.

## Format

```
<type>[optional scope]: <description>

[optional body]

[optional footer(s)]
```

## Types and Version Impact

| Type | Version Bump | Use When |
|------|-------------|----------|
| `feat` | **minor** (0.23.2 -> 0.24.0) | Adding new functionality |
| `fix` | **patch** (0.23.2 -> 0.23.3) | Fixing a bug |
| `feat!` or `fix!` | **major** (0.23.2 -> 1.0.0) | Breaking change (the `!` suffix) |
| `chore` | none | Maintenance, deps, CI - no release |
| `docs` | none | Documentation only |
| `refactor` | none | Code change that neither fixes nor adds |
| `test` | none | Adding or updating tests |
| `perf` | none | Performance improvement (no API change) |
| `ci` | none | CI/CD workflow changes |

Only `feat` and `fix` trigger a new release. Other types appear in CHANGELOG but don't bump the version.

## Scopes (optional)

Scopes narrow down what changed. Use the module name:

```
feat(git): add rebase subcommand support
fix(cargo): preserve exit code on test failures
feat(kubectl): add logs subcommand
chore(ci): update rust-toolchain to 1.82
docs(readme): add Python command examples
```

Common scopes: `git`, `cargo`, `grep`, `gh`, `pnpm`, `vitest`, `playwright`, `prisma`, `tsc`, `next`, `lint`, `prettier`, `ruff`, `pytest`, `pip`, `go`, `golangci`, `kubectl`, `docker`, `gain`, `init`, `hook`, `ci`, `tee`, `read`, `find`, `ls`.

## Breaking Changes

Two ways to signal a breaking change (bumps major version):

```
# Option 1: ! suffix on type
feat!: remove --depth and --format flags from ls command

# Option 2: BREAKING CHANGE footer
feat(ls): rewrite output format

BREAKING CHANGE: Removes --depth, --format (tree/flat/json) flags.
Use rtk tree for directory tree views instead.
```

## Examples

### New filter command
```
feat(wc): add rtk wc command for compact word/line/byte counts
```

### Bug fix with scope
```
fix(git): support multiple -m flags in git commit
```

### Performance improvement (no release)
```
perf(grep): compile regex once with lazy_static
```

### Chore - dependency update (no release)
```
chore(deps): bump clap from 4.5 to 4.6
```

### Multi-line with body
```
feat(tee): save raw output to file for LLM re-read

When a filtered command fails, tee the full unfiltered output
to ~/.local/share/rtk/tee/ so the LLM can read it without
re-running the command.
```

### Breaking change with explanation
```
feat!(ls): convert from reimplementation to native proxy

BREAKING CHANGE: rtk ls no longer accepts --depth, --format,
or --sort flags. It now proxies to native ls with output
filtering. Use rtk tree for tree-style directory views.
```

## Release Please Flow

1. You push conventional commits to master
2. Release Please reads commit types and creates/updates a PR:
   - Bumps version in `Cargo.toml` and `Cargo.lock`
   - Updates `.release-please-manifest.json`
   - Generates `CHANGELOG.md` entries from commit messages
3. You review and merge the Release Please PR
4. Release Please creates a git tag (e.g., `v0.24.0`)
5. `release.yml` builds binaries and pushes to Homebrew

## Rules

- **Never manually edit** version in `Cargo.toml` - Release Please owns it
- **Never manually create** version tags - Release Please creates them
- **Never manually edit** `.release-please-manifest.json` (except to fix sync issues)
- **Always use conventional prefixes** - commits without them are ignored by Release Please
- **Squash merges** should preserve the conventional commit format in the squash message
- **PR titles** should follow conventional commit format (GitHub squash uses PR title as commit message)
