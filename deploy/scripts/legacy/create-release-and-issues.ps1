$ErrorActionPreference = "Stop"

param(
  [string]$Owner = "",
  [string]$Repo = "",
  [string]$Tag = "v0.1.0-alpha",
  [string]$Token = ""
)

if (-not $Owner -or -not $Repo) {
  Write-Error "Owner and Repo are required, e.g. -Owner my-org -Repo autoloop-app"
}

if (-not $Token) {
  if ($env:GITHUB_TOKEN) {
    $Token = $env:GITHUB_TOKEN
  } else {
    Write-Error "GitHub token is required. Pass -Token or set GITHUB_TOKEN."
  }
}

$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$releaseNotes = Join-Path $repoRoot "RELEASE_NOTES_v0.1.0-alpha.md"
$issueBacklog = Join-Path $repoRoot "docs\ISSUE_BACKLOG_v0.1.0-alpha.md"

$headers = @{
  Authorization = "Bearer $Token"
  Accept        = "application/vnd.github+json"
  "X-GitHub-Api-Version" = "2022-11-28"
}

if (-not (Test-Path $releaseNotes)) {
  Write-Error "Release notes file not found: $releaseNotes"
}

if (-not (Test-Path $issueBacklog)) {
  Write-Error "Issue backlog file not found: $issueBacklog"
}

$releaseBody = Get-Content $releaseNotes -Raw

# Create release (idempotent-ish: continue if already exists)
try {
  $payload = @{
    tag_name   = $Tag
    name       = $Tag
    body       = $releaseBody
    draft      = $false
    prerelease = $true
  } | ConvertTo-Json -Depth 8
  Invoke-RestMethod -Method Post -Uri "https://api.github.com/repos/$Owner/$Repo/releases" -Headers $headers -Body $payload
  Write-Host "Release created: $Tag"
} catch {
  Write-Warning "Release create failed (it may already exist): $($_.Exception.Message)"
}

# Parse issues from markdown sections beginning with "## "
$raw = Get-Content $issueBacklog -Raw
$blocks = $raw -split "(?m)^## "
$issueCount = 0
foreach ($block in $blocks) {
  if (-not $block.Trim()) { continue }
  $lines = $block -split "`r?`n"
  $title = $lines[0].Trim()
  if (-not $title) { continue }
  $body = ($lines[1..($lines.Length - 1)] -join "`n").Trim()
  if (-not $body) { continue }
  $issuePayload = @{
    title = $title
    body  = $body
  } | ConvertTo-Json -Depth 8
  try {
    Invoke-RestMethod -Method Post -Uri "https://api.github.com/repos/$Owner/$Repo/issues" -Headers $headers -Body $issuePayload | Out-Null
    $issueCount += 1
  } catch {
    Write-Warning "Issue create failed for [$title]: $($_.Exception.Message)"
  }
}

Write-Host "Issue creation attempted for $issueCount entries."

