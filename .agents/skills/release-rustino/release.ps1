# release.ps1 - Automated Release Script for Rustino
#
#   release.ps1 -Version 0.5.6
#
# The version is not stored in the repository: the release workflow gets it from here.

param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$')]
    [string]$Version
)

$ErrorActionPreference = "Stop"
$newVersion = $Version

Write-Host "=== Starting Rustino Release Process ===" -ForegroundColor Cyan

# 1. Check if git workspace is clean
$status = git status --porcelain
if ($status) {
    Write-Error "Git working directory is not clean. Please commit or stash your changes before running this script."
}

# 2. Check current branch is development
$currentBranch = git branch --show-current
if ($currentBranch -ne "development") {
    Write-Host "Current branch is '$currentBranch'. Switching to 'development'..." -ForegroundColor Yellow
    git checkout development
}

# 3. Pull latest development
Write-Host "Pulling latest changes on development..." -ForegroundColor Gray
git pull origin development

# 4. Create PR from development to main
Write-Host "Creating Pull Request from development into main..." -ForegroundColor Gray
$prUrl = gh pr create --base main --head development --title "Release: Merge development into main (v$newVersion)" --body "Automated release PR for version $newVersion."
if (-not $prUrl) {
    Write-Error "Failed to create Pull Request."
}
Write-Host "Created PR: $prUrl" -ForegroundColor Green

# 5. Post the required comment for Claude review (user rule)
Write-Host "Requesting Claude review on the PR..." -ForegroundColor Gray
gh pr comment $prUrl --body "@claude review this"

# 6. Check mergeability and wait if needed
Write-Host "Waiting for PR mergeability checks..." -ForegroundColor Gray
$attempts = 0
$maxAttempts = 12 # 60 seconds total
$isMergeable = $false

while ($attempts -lt $maxAttempts) {
    Start-Sleep -Seconds 5
    $prInfo = gh pr view $prUrl --json mergeable | ConvertFrom-Json
    $mergeableState = $prInfo.mergeable
    
    Write-Host "Mergeability state: $mergeableState" -ForegroundColor Gray
    
    if ($mergeableState -eq "MERGEABLE") {
        $isMergeable = $true
        break
    } elseif ($mergeableState -eq "CONFLICTING") {
        Write-Error "PR has merge conflicts! Please resolve conflicts manually and finish the release."
    }
    
    $attempts++
}

if (-not $isMergeable) {
    Write-Error "PR mergeability check timed out or is not mergeable. Please merge manually."
}

# 7. Merge development into main
Write-Host "Merging PR into main..." -ForegroundColor Gray
gh pr merge $prUrl --merge --delete-branch=$false
Write-Host "Successfully merged PR into main!" -ForegroundColor Green

# 8. Sync main back into development
Write-Host "Synchronizing branches (merging main back into development)..." -ForegroundColor Gray
git checkout main
git pull origin main
git checkout development
git merge main --no-ff -m "Merge branch 'main' into development to sync [skip ci]"
git push origin development

# 9. Trigger release
Write-Host "Triggering NuGet release workflow on main branch for version $newVersion..." -ForegroundColor Gray
gh workflow run release.yml --ref main -f version=$newVersion
Start-Sleep -Seconds 3
gh run list --workflow=release.yml --limit 1

Write-Host "=== Rustino Release Process Completed Successfully ===" -ForegroundColor Green
