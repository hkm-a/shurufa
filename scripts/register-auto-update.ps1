# register-auto-update.ps1 — 注册 Windows 计划任务：定期自动检查/安装更新
#
# 用法：
#   pwsh -File scripts\register-auto-update.ps1 `
#     -ManifestUrl https://example.com/update.json `
#     -Channel stable -Apply -Silent
#
# 默认每天 10:00 运行；-Apply 表示命中灰度后自动安装。

param(
  [Parameter(Mandatory = $true)][string]$ManifestUrl,
  [string]$Channel = 'stable',
  [switch]$Apply,
  [switch]$Silent,
  [string]$TaskName = 'ShurufaAutoUpdate',
  [int]$Hour = 10,
  [int]$Minute = 0
)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$script = Join-Path $repo 'scripts\auto-update.ps1'

$args = @('-ManifestUrl', $ManifestUrl, '-Channel', $Channel)
if ($Apply) { $args += '-Apply' }
if ($Silent) { $args += '-Silent' }

$action = New-ScheduledTaskAction -Execute 'pwsh.exe' -Argument "-NoProfile -ExecutionPolicy Bypass -File `"$script`" $($args -join ' ')"
$trigger = New-ScheduledTaskTrigger -Daily -At "$Hour`:$Minute"
$settings = New-ScheduledTaskSettingsSet -StartWhenAvailable -DontStopOnIdleEnd
Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Settings $settings -Force | Out-Null
Write-Host "已注册计划任务：$TaskName（每天 $Hour`:$Minute，channel=$Channel apply=$Apply silent=$Silent）"
