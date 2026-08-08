[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$InstallDir,
    [int]$TimeoutSeconds = 12
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Deploy-Shurufa.ps1')

# 安装终态验证：确认"输入法后台持续运行"所需的全部条件已就绪。
# 供安装器收尾调用，也可随时手动运行自检。退出码 0 表示全部通过。
if (Test-ShurufaInstallComplete -Dir $InstallDir -TimeoutSeconds $TimeoutSeconds) {
    exit 0
}
exit 1
