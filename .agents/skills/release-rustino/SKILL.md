---
name: release-rustino
description: Switches to latest development branch, creates a PR from development into main with @claude review, merges if no conflicts, syncs main back to development, and triggers the NuGet release for the given version.
---

# Rustino Release Automator

This skill automates the release preparation, branch merging, branch synchronization, and NuGet deployment process for the Rustino repository.

## Invocation

```bash
/release-rustino <version>   # e.g. /release-rustino 0.5.6
```

The version is not stored in the repository: ask for it when it is missing (the latest release is the
newest `v*` tag).

## What This Skill Does

1. **Ensures the workspace is clean** and checks out the `development` branch.
2. **Pulls the latest updates** from `origin/development`.
3. **Creates a Pull Request** from `development` into `main` using the GitHub CLI (`gh`).
4. **Requests review from @claude** bot on the PR by posting a comment with `@claude review this`.
5. **Waits for mergeability checks** to ensure there are no conflicts.
6. **Merges the PR** into `main`.
7. **Synchronizes the branches** by switching to `main`, pulling, switching to `development`, merging `main`, and pushing.
8. **Triggers the GitHub release workflow** (`release.yml`) on `main` with the version to publish.

## Prerequisites

- **GitHub CLI** (`gh`) must be installed and authenticated.
- **PowerShell 7** (`pwsh`) or Windows PowerShell must be available.

## Step-by-Step Execution

You can execute the entire release flow automatically by running:

```powershell
powershell -File .agents/skills/release-rustino/release.ps1 -Version 0.5.6
```

If you prefer to run it manually or need to troubleshoot a step, follow the steps below:

### Phase 1 — Switch to development
```powershell
git checkout development
git pull origin development
```

### Phase 2 — Create PR and Request Review
Create the Pull Request and comment for Claude review:
```powershell
gh pr create --base main --head development --title "Release: Merge development into main (v0.5.6)" --body "Automated release PR."
# Note down the PR URL/number returned, then comment:
gh pr comment <PR_URL> --body "@claude review this"
```

### Phase 3 — Verify and Merge PR
Ensure no conflicts exist, then merge:
```powershell
gh pr merge <PR_URL> --merge --delete-branch=false
```

### Phase 4 — Sync branches
```powershell
git checkout main
git pull origin main
git checkout development
git merge main --no-ff -m "Merge branch 'main' into development to sync [skip ci]"
git push origin development
```

### Phase 5 — Trigger Release
```powershell
gh workflow run release.yml --ref main -f version=0.5.6
# Check status:
gh run list --workflow=release.yml --limit 1
```
