[CmdletBinding()]
param(
    [string]$InstallDir,
    [string]$StatePath,
    [switch]$Restore,
    [switch]$Remove
)

$ErrorActionPreference = 'Stop'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$runValueName = 'shurufa-host'

if ($Remove) {
    Remove-ItemProperty -LiteralPath $runKey -Name $runValueName -ErrorAction SilentlyContinue
    exit 0
}

if ($Restore) {
    if (-not (Test-Path -LiteralPath $StatePath -PathType Leaf)) {
        exit 0
    }
    $state = Get-Content -LiteralPath $StatePath -Raw | ConvertFrom-Json
    if ($state.hasValue) {
        New-Item -Path $runKey -Force | Out-Null
        New-ItemProperty -LiteralPath $runKey -Name $runValueName -Value $state.value -PropertyType String -Force | Out-Null
    }
    else {
        Remove-ItemProperty -LiteralPath $runKey -Name $runValueName -ErrorAction SilentlyContinue
    }
    exit 0
}

if ([string]::IsNullOrWhiteSpace($InstallDir) -or [string]::IsNullOrWhiteSpace($StatePath)) {
    throw '安装目录与启动项状态文件均不能为空。'
}

$hostPath = Join-Path $InstallDir 'shurufa-host.exe'
if (-not (Test-Path -LiteralPath $hostPath -PathType Leaf)) {
    throw "后台宿主不存在：$hostPath"
}
$expectedValue = '"{0}" supervise' -f $hostPath
$existing = Get-ItemProperty -LiteralPath $runKey -Name $runValueName -ErrorAction SilentlyContinue
$existingValue = $existing.PSObject.Properties[$runValueName]
$stateDirectory = Split-Path -Parent $StatePath
if ($stateDirectory) {
    New-Item -ItemType Directory -Path $stateDirectory -Force | Out-Null
}
@{
    hasValue = $null -ne $existingValue
    value = if ($null -eq $existingValue) { $null } else { [string]$existingValue.Value }
} | ConvertTo-Json -Compress | Set-Content -LiteralPath $StatePath -NoNewline -Encoding utf8

New-Item -Path $runKey -Force | Out-Null
New-ItemProperty -LiteralPath $runKey -Name $runValueName -Value $expectedValue -PropertyType String -Force | Out-Null
$registered = (Get-ItemProperty -LiteralPath $runKey -Name $runValueName).PSObject.Properties[$runValueName].Value
if ($registered -ne $expectedValue) {
    throw 'Windows 未确认 Shurufa 后台服务的登录启动配置。'
}
