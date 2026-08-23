# generate-update-manifest.ps1 — 生成 dist/update.json
#
# 用法：
#   pwsh -File scripts/generate-update-manifest.ps1 `
#     -StableVersion 0.8.0 -StableUrl https://example.com/FOX-Setup-0.8.0.exe `
#     -StableSha256 abc... [-StableRollout 100] `
#     [-CanaryVersion 0.8.0-canary.abc123 -CanaryUrl https://example.com/canary.exe -CanarySha256 def... -CanaryRollout 10]
#
# 不传 Canary 参数时只输出 stable 通道。

param(
  [Parameter(Mandatory = $true)][string]$StableVersion,
  [Parameter(Mandatory = $true)][string]$StableUrl,
  [string]$StableSha256 = '',
  [int]$StableRollout = 100,
  [string]$CanaryVersion = '',
  [string]$CanaryUrl = '',
  [string]$CanarySha256 = '',
  [int]$CanaryRollout = 10
)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$dist = Join-Path $repo 'dist'
New-Item -ItemType Directory -Path $dist -Force | Out-Null

$channels = [ordered]@{
  stable = [ordered]@{
    version        = $StableVersion
    url            = $StableUrl
    sha256         = $StableSha256
    rollout_percent = $StableRollout
  }
}

if ($CanaryVersion -and $CanaryUrl) {
  $channels['canary'] = [ordered]@{
    version        = $CanaryVersion
    url            = $CanaryUrl
    sha256         = $CanarySha256
    rollout_percent = $CanaryRollout
  }
}

$manifest = [ordered]@{ channels = $channels }
$out = Join-Path $dist 'update.json'
$manifest | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $out -Encoding utf8
Write-Host "已生成：$out"
Get-Content $out
