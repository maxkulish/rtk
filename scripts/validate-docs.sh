#!/bin/bash
set -e

echo "Validating RTK documentation consistency..."

# 1. Version: Cargo.toml must match all doc files
CARGO_VERSION=$(grep '^version = ' Cargo.toml | cut -d'"' -f2)
echo "Cargo.toml version: $CARGO_VERSION"

ERRORS=0

for file in README.md CLAUDE.md ARCHITECTURE.md; do
  if [ ! -f "$file" ]; then
    echo "  $file not found, skipping"
    continue
  fi
  if ! grep -q "$CARGO_VERSION" "$file"; then
    echo "ERROR: $file does not mention version $CARGO_VERSION"
    ERRORS=$((ERRORS + 1))
  fi
  if ! grep -q "x-release-please-version" "$file"; then
    echo "ERROR: $file missing x-release-please-version annotation"
    echo "  Add <!-- x-release-please-version --> on the line with the version string"
    echo "  This lets Release Please auto-update the version on release"
    ERRORS=$((ERRORS + 1))
  fi
done

if [ "$ERRORS" -gt 0 ]; then
  echo ""
  echo "FAILED: $ERRORS version/annotation errors found"
  echo ""
  echo "Fix: Release Please auto-updates version strings via x-release-please-version"
  echo "annotations. Each doc file with a hardcoded version must have the annotation."
  echo "See release-please-config.json extra-files and docs for details."
  exit 1
fi
echo "OK: Version consistency - all docs mention $CARGO_VERSION with release-please annotations"

# 2. Module count: main.rs must match ARCHITECTURE.md
MAIN_MODULES=$(grep -c '^mod ' src/main.rs)
echo "Module count in main.rs: $MAIN_MODULES"

if [ -f "ARCHITECTURE.md" ]; then
  ARCH_MODULES=$(grep 'Total:.*modules' ARCHITECTURE.md | grep -o '[0-9]\+' | head -1)
  if [ -z "$ARCH_MODULES" ]; then
    echo "  Could not extract module count from ARCHITECTURE.md"
  else
    echo "Module count in ARCHITECTURE.md: $ARCH_MODULES"
    if [ "$MAIN_MODULES" != "$ARCH_MODULES" ]; then
      echo "ERROR: Module count mismatch: main.rs=$MAIN_MODULES, ARCHITECTURE.md=$ARCH_MODULES"
      echo "  Update the 'Total: N modules' line in ARCHITECTURE.md"
      exit 1
    fi
  fi
fi
echo "OK: Module count consistent ($MAIN_MODULES modules)"

# 3. Python/Go commands must be documented
PYTHON_GO_CMDS=("ruff" "pytest" "pip" "go" "golangci")
echo "Checking Python/Go commands documentation..."

for cmd in "${PYTHON_GO_CMDS[@]}"; do
  for file in README.md CLAUDE.md; do
    if [ ! -f "$file" ]; then
      continue
    fi
    if ! grep -q "$cmd" "$file"; then
      echo "ERROR: $file does not mention command: $cmd"
      exit 1
    fi
  done
done
echo "OK: All Python/Go commands documented in README.md and CLAUDE.md"

# 4. Hook file must rewrite Python/Go commands
HOOK_FILE=".claude/hooks/rtk-rewrite.sh"
if [ -f "$HOOK_FILE" ]; then
  echo "Checking hook rewrites..."
  for cmd in "${PYTHON_GO_CMDS[@]}"; do
    if ! grep -q "$cmd" "$HOOK_FILE"; then
      echo "  Warning: Hook may not rewrite $cmd (verify manually)"
    fi
  done
  echo "OK: Hook file exists and mentions Python/Go commands"
else
  echo "  Warning: Hook file not found: $HOOK_FILE"
fi

echo ""
echo "Documentation validation passed"
