[CmdletBinding()]
param(
    [string]$InstallDir = (Join-Path $env:ProgramData 'shurufa'),
    [switch]$Remove
)

$ErrorActionPreference = 'Stop'
# 先保存：Deploy-Shurufa.ps1 点源时会用自己的同名 $InstallDir 默认值覆盖当前值
$InstalledDir = $InstallDir
. (Join-Path $PSScriptRoot 'Deploy-Shurufa.ps1')

if ($Remove) {
    Unregister-ShurufaStartup
    exit 0
}

Register-ShurufaStartup -Dir $InstalledDir
Write-Output "登录自启动项已注册：$(Resolve-ShurufaHostSuperviseCommand -Dir $InstalledDir)"
