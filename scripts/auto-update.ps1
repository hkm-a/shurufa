# auto-update.ps1 — 自动更新入口（供计划任务/托盘调用）
#
# 用法：
#   pwsh -File scripts\auto-update.ps1 `
#     -ManifestUrl https://example.com/update.json `
#     -Channel stable `
#     -Apply -Silent
#
# 不传 -Apply 时只检查并输出结果；传 -Apply 时在灰度命中后自动下载安装。

param(
  [Parameter(Mandatory = $true)][string]$ManifestUrl,
  [string]$Channel = 'stable',
  [string]$CtlPath = '',
  [switch]$Apply,
  [switch]$Silent
)

$ErrorActionPreference = 'Stop'
if (-not $CtlPath) {
  $candidates = @(
    (Join-Path $env:ProgramData 'shurufa\shurufa-ctl.exe'),
    (Join-Path $PSScriptRoot '..\target\debug\shurufa-ctl.exe')
  )
  $CtlPath = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $CtlPath -or -not (Test-Path $CtlPath)) {
  Write-Error "未找到 shurufa-ctl.exe，请用 -CtlPath 指定"
  exit 3
}

$commonArgs = @('update', '--url', $ManifestUrl, '--channel', $Channel, '--check-only')
$check = & $CtlPath @commonArgs 2>&1
$code = $LASTEXITCODE
$check | ForEach-Object { Write-Host $_ }

if ($code -eq 0 -and $Apply) {
  Write-Host '灰度命中，开始自动更新…'
  $applyArgs = @('update', '--url', $ManifestUrl, '--channel', $Channel)
  if ($Silent) { $applyArgs += '--silent' }
  & $CtlPath @applyArgs
  exit $LASTEXITCODE
}

exit $code
