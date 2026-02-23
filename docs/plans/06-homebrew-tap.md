# Homebrew Tap Setup: User-Level Multi-Package Tap

## Goal

Create a **user-level Homebrew tap** (`maxkulish/homebrew-tap`) that serves as a central distribution point for all your packages — shared with your team. RTK is the first formula; more can be added later.

Users install any package via:
```bash
brew tap maxkulish/tap
brew install rtk        # first package
brew install <other>    # future packages
```

---

## Current State (updated 2026-02-23)

- **Tap repo**: `maxkulish/homebrew-tap` created (public), `Formula/` dir + README pushed to `main`
- **release.yml**: Updated — checksums, formula URLs, push target, auth, base64, validation, failure alert all applied
- **Remaining**: PAT creation (Step 2), secret setup (Step 3), end-to-end test (Steps 5/12-14)

---

## Step 1: Create the Tap Repository

The repo **must** be named `homebrew-tap` (Homebrew convention: `homebrew-<suffix>` maps to `brew tap <user>/<suffix>`).

```bash
gh repo create maxkulish/homebrew-tap \
  --public \
  --description "Homebrew tap for maxkulish packages"
```

Then set up the directory structure:

```bash
git clone git@github.com:maxkulish/homebrew-tap.git
cd homebrew-tap

mkdir -p Formula

cat > README.md << 'EOF'
# Homebrew Tap

Custom Homebrew formulae maintained by maxkulish.

## Usage

```bash
brew tap maxkulish/tap
brew install <formula>
```

## Available Formulae

| Formula | Description | Install |
|---------|-------------|---------|
| rtk | Rust Token Killer — CLI proxy to minimize LLM token consumption | `brew install maxkulish/tap/rtk` |
EOF

git add .
git commit -m "Initial tap structure"
git push origin main
```

### Why `homebrew-tap` (not `homebrew-rtk`)

- One tap for all your tools — no repo sprawl
- Team members run `brew tap maxkulish/tap` once, then get all packages
- Standard convention used by most orgs (e.g., `hashicorp/tap`, `goreleaser/tap`)

---

## Step 2: Create a GitHub Personal Access Token (Fine-Grained)

1. Go to: https://github.com/settings/tokens?type=beta
2. Create a token:
   - **Name**: `HOMEBREW_TAP_TOKEN`
   - **Expiration**: 90 days (or longer, note the renewal date)
   - **Repository access**: Select repositories → `maxkulish/homebrew-tap`
   - **Permissions**:
     - **Contents**: Read and write (to push formula files)
     - **Metadata**: Read (required by GitHub)
3. Copy the token value

**Long-term alternative**: Consider a **GitHub App** instead of a PAT. GitHub Apps generate tokens dynamically in Actions without expiration. This eliminates the renewal burden entirely (see Blind Spot #4 below).

---

## Step 3: Add the Secret to the RTK Repo

```bash
gh secret set HOMEBREW_TAP_TOKEN --repo maxkulish/rtk
# Paste the token when prompted
```

Any future project that publishes to this tap will also need this secret (or a separate token with access to `homebrew-tap`).

---

## Step 4: Update release.yml

### 4a. Change the checksums download repo (line 206)

```diff
       - name: Download checksums
         run: |
           gh release download "${{ steps.version.outputs.tag }}" \
-            --repo rtk-ai/rtk \
+            --repo ${{ github.repository }} \
             --pattern checksums.txt
```

Using `${{ github.repository }}` makes it fork-safe — works on both `maxkulish/rtk` and `rtk-ai/rtk`.

### 4b. Update formula download URLs (lines 229-240)

In the `Generate formula` step, change all `url` lines from `rtk-ai/rtk` to use `github.repository`:

```diff
-              url "https://github.com/rtk-ai/rtk/releases/download/TAG_PLACEHOLDER/rtk-aarch64-apple-darwin.tar.gz"
+              url "https://github.com/REPO_PLACEHOLDER/releases/download/TAG_PLACEHOLDER/rtk-aarch64-apple-darwin.tar.gz"
```

(Repeat for all 4 platform URLs)

Then add a sed replacement:

```diff
           sed -i "s/TAG_PLACEHOLDER/${{ steps.version.outputs.tag }}/g" rtk.rb
+          sed -i "s|REPO_PLACEHOLDER|${{ github.repository }}|g" rtk.rb
```

### 4c. Change the push target and fix authentication (lines 279-295)

The `GH_TOKEN` env var must be set explicitly — the default `GITHUB_TOKEN` only has access to the current repo (`maxkulish/rtk`), not the tap repo.

```diff
       - name: Push to homebrew-tap
+        env:
+          GH_TOKEN: ${{ secrets.HOMEBREW_TAP_TOKEN }}
         run: |
-          CONTENT=$(base64 -w 0 rtk.rb)
+          CONTENT=$(cat rtk.rb | base64 | tr -d '\n')
-          SHA=$(gh api repos/rtk-ai/homebrew-tap/contents/Formula/rtk.rb --jq '.sha' 2>/dev/null || echo "")
+          SHA=$(gh api repos/maxkulish/homebrew-tap/contents/Formula/rtk.rb --jq '.sha' 2>/dev/null || echo "")
           if [ -n "$SHA" ]; then
-            gh api -X PUT repos/rtk-ai/homebrew-tap/contents/Formula/rtk.rb \
+            gh api -X PUT repos/maxkulish/homebrew-tap/contents/Formula/rtk.rb \
               -f message="rtk ${{ steps.version.outputs.version }}" \
               -f content="$CONTENT" \
               -f sha="$SHA"
           else
-            gh api -X PUT repos/rtk-ai/homebrew-tap/contents/Formula/rtk.rb \
+            gh api -X PUT repos/maxkulish/homebrew-tap/contents/Formula/rtk.rb \
               -f message="rtk ${{ steps.version.outputs.version }}" \
               -f content="$CONTENT"
           fi
-        env:
-          GH_TOKEN: ${{ secrets.HOMEBREW_TAP_TOKEN }}
```

Key fixes in this step:
- **`GH_TOKEN`** explicitly set to `HOMEBREW_TAP_TOKEN` (cross-repo auth)
- **`base64 | tr -d '\n'`** instead of `base64 -w 0` (cross-platform safe — `-w 0` is GNU-only, fails on macOS runners)

### 4d. Add formula validation before push (new step)

Insert this step between "Generate formula" and "Push to homebrew-tap":

```yaml
      - name: Validate formula
        run: |
          ruby -c rtk.rb
          echo "Formula syntax OK"
```

This catches broken Ruby syntax from sed replacements before pushing to the tap. A full `brew audit` requires Homebrew installed on the runner — `ruby -c` is lightweight and sufficient for catching sed-induced syntax errors.

### 4e. Add failure notification (new step)

Add after the push step to catch silent failures (e.g., expired token):

```yaml
      - name: Alert on homebrew push failure
        if: failure()
        run: |
          gh issue create \
            --repo ${{ github.repository }} \
            --title "Homebrew formula push failed for ${{ steps.version.outputs.tag }}" \
            --body "The homebrew job failed. Check if HOMEBREW_TAP_TOKEN has expired. Run: gh workflow run release.yml -f tag=${{ steps.version.outputs.tag }}" \
            --label "ci"
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

---

## Step 5: Verify the Automated Pipeline

The full flow after setup:

```
push to master
  → release-please creates version PR
    → merge PR → git tag + GitHub Release
      → release.yml builds 5 platform binaries
        → uploads to GitHub Release with checksums.txt
          → homebrew job:
              1. Downloads checksums.txt
              2. Generates rtk.rb formula with correct SHA256s
              3. Validates Ruby syntax (ruby -c)
              4. Pushes rtk.rb to maxkulish/homebrew-tap via GitHub API
              5. On failure → creates GitHub issue as alert
```

### Test with manual workflow dispatch

```bash
# Trigger release workflow against an existing tag
gh workflow run release.yml --repo maxkulish/rtk -f tag=v0.23.2
```

### Test the installation

```bash
brew tap maxkulish/tap
brew install rtk
rtk --version   # Should show 0.23.2
rtk gain         # Should work (confirms correct binary)
```

### Test upgrade flow

After a new release is pushed:
```bash
brew update
brew upgrade rtk
```

---

## Step 6: Adding Future Packages

To add another formula to the same tap (e.g., `mycli`):

1. In `mycli`'s CI, add the same `HOMEBREW_TAP_TOKEN` secret
2. Generate `mycli.rb` formula in CI
3. Push to `maxkulish/homebrew-tap/contents/Formula/mycli.rb` via same API pattern
4. Users install: `brew install maxkulish/tap/mycli`

No additional `brew tap` needed — once tapped, all formulae are available.

---

## Step 7: Team Sharing

Share with your team:

```bash
# One-time setup (each team member)
brew tap maxkulish/tap

# Install any tool
brew install rtk

# Stay updated
brew update && brew upgrade
```

For private repos (if needed later):
- Make `homebrew-tap` private
- Team members need a GitHub token: `HOMEBREW_GITHUB_API_TOKEN=<token> brew tap maxkulish/tap`
- Or use `gh auth` + `HOMEBREW_GITHUB_API_TOKEN=$(gh auth token)`

---

## Blind Spots Addressed

### 1. Cross-Repo Authentication (Critical)

**Problem**: `gh api` uses `GITHUB_TOKEN` by default, which can only access the current repo. Pushing to `homebrew-tap` fails silently with a 403.

**Fix**: Explicit `GH_TOKEN: ${{ secrets.HOMEBREW_TAP_TOKEN }}` env var on the push step (Step 4c).

### 2. Cross-Platform base64 Incompatibility

**Problem**: `base64 -w 0` is a GNU flag (Linux). On macOS runners, BSD `base64` doesn't support `-w` and the step fails.

**Fix**: `cat rtk.rb | base64 | tr -d '\n'` — works on both GNU and BSD (Step 4c). The homebrew job runs on `ubuntu-latest` today, but this makes it portable if that changes.

### 3. Missing Formula Validation

**Problem**: sed replacements can break Ruby syntax. A broken formula pushed to the tap causes `brew update` failures for all users.

**Fix**: `ruby -c rtk.rb` validation step before pushing (Step 4d). Lightweight — doesn't require Homebrew on the runner.

### 4. Token Expiration & Silent Failures

**Problem**: Fine-grained PATs expire. When they do, the homebrew push fails but the release itself succeeds — you don't notice the tap is stale.

**Fixes**:
- **Short-term**: `if: failure()` step creates a GitHub issue automatically (Step 4e)
- **Long-term**: Replace PAT with a **GitHub App**:
  1. Create a GitHub App in your account settings
  2. Grant it Contents write access to `homebrew-tap`
  3. Install the app on `homebrew-tap` repo
  4. Use `tibdex/github-app-token` action to generate tokens dynamically
  5. Tokens never expire — the app generates fresh ones per workflow run

### 5. macOS Gatekeeper & Unsigned Binaries

**Problem**: macOS may quarantine unsigned binaries downloaded from GitHub Releases, showing "cannot be opened because the developer cannot be verified."

**Status**: Homebrew strips the quarantine attribute (`xattr -d com.apple.quarantine`) during `brew install`, so this is a non-issue for tap users. It only affects users who download `.tar.gz` manually.

**If manual downloads become common**: Add a caveats block to the formula:
```ruby
def caveats
  on_macos do
    <<~EOS
      If macOS blocks rtk, run:
        xattr -d com.apple.quarantine $(which rtk)
    EOS
  end
end
```

Or, for full fix: sign macOS binaries with `codesign` in CI (requires Apple Developer account + certificate in GitHub secrets). This is overkill for now.

---

## Checklist

| # | Action | Status |
|---|--------|--------|
| 1 | Create `maxkulish/homebrew-tap` repo on GitHub | [x] done 2026-02-23 |
| 2 | Add `Formula/` dir, README, push to main | [x] done 2026-02-23 |
| 3 | Create fine-grained PAT with Contents write on `homebrew-tap` | [ ] manual — https://github.com/settings/tokens?type=beta |
| 4 | Add `HOMEBREW_TAP_TOKEN` secret to `maxkulish/rtk` | [ ] manual — `gh secret set HOMEBREW_TAP_TOKEN --repo maxkulish/rtk` |
| 5 | Update `release.yml` — checksums download (use `github.repository`) | [x] done 2026-02-23 |
| 6 | Update `release.yml` — formula URLs (use `github.repository`) | [x] done 2026-02-23 |
| 7 | Update `release.yml` — push target (`maxkulish/homebrew-tap`) | [x] done 2026-02-23 |
| 8 | Update `release.yml` — explicit `GH_TOKEN` for cross-repo auth | [x] done 2026-02-23 |
| 9 | Update `release.yml` — cross-platform `base64` command | [x] done 2026-02-23 |
| 10 | Update `release.yml` — add `ruby -c` formula validation step | [x] done 2026-02-23 |
| 11 | Update `release.yml` — add failure alert step (auto-create issue) | [x] done 2026-02-23 |
| 12 | Test with `gh workflow run release.yml -f tag=v0.23.2` | [ ] blocked by #3, #4 |
| 13 | Verify `brew tap maxkulish/tap && brew install rtk` works | [ ] blocked by #12 |
| 14 | Document tap URL in project README | [ ] |

---

## Notes

- **Naming**: `homebrew-tap` is the standard name for multi-package taps. `brew tap maxkulish/tap` strips the `homebrew-` prefix automatically.
- **Public vs Private**: Keep the tap public for easy team access. Private taps require token auth for every `brew update`.
- **Token renewal**: Fine-grained PATs expire. The failure alert step (4e) will catch this, but consider migrating to a GitHub App long-term.
- **Formula format**: Homebrew prefers one `.rb` file per formula in `Formula/`. No nested directories.
- **Runner OS**: The homebrew job runs on `ubuntu-latest`. The cross-platform base64 fix makes it safe to move to any runner if needed.
