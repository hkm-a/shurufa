[CmdletBinding()]
param(
    [ValidateSet('Debug', 'Release')]
    [string]$Configuration = 'Release',
    [switch]$SkipBuild,
    [switch]$NoStartHost
)

$ErrorActionPreference = 'Stop'

function Assert-Administrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw '安装需要管理员权限。请在 PowerShell 7 中以管理员身份重新运行本脚本。'
    }
}

function Invoke-Native([string]$File, [string[]]$Arguments) {
    & $File @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "命令失败（退出码 $LASTEXITCODE）：$File $($Arguments -join ' ')"
    }
}

Assert-Administrator
$sourceRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$targetDir = Join-Path $sourceRoot ('target\' + $Configuration.ToLowerInvariant())

if (-not $SkipBuild) {
    $cargoArgs = @('build', '-p', 'shurufa-tsf', '-p', 'shurufa-algo', '-p', 'shurufa-host')
    if ($Configuration -eq 'Release') {
        $cargoArgs += '--release'
    }
    Push-Location $sourceRoot
    try {
        Invoke-Native 'cargo' $cargoArgs
        if ($Configuration -eq 'Release') {
            Invoke-Native 'npm' @('--prefix', 'platforms/windows-settings', 'run', 'tauri', '--', 'build', '--no-bundle')
        }
        else {
            Invoke-Native 'cargo' @('build', '-p', 'shurufa-settings')
        }
    }
    finally {
        Pop-Location
    }
}

$required = @(
    (Join-Path $targetDir 'shurufa_tsf.dll'),
    (Join-Path $targetDir 'shurufa-algo.exe'),
    (Join-Path $targetDir 'shurufa-clipd.exe'),
    (Join-Path $targetDir 'shurufa-ui.exe'),
    (Join-Path $targetDir 'shurufa-ctl.exe'),
    (Join-Path $targetDir 'Shurufa.exe'),
    (Join-Path $sourceRoot 'third_party\librime\dist\lib\rime.dll'),
    (Join-Path $sourceRoot 'third_party\librime\dist\bin\rime_deployer.exe')
)
foreach ($file in $required) {
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) {
        throw "缺少安装文件：$file"
    }
}

# 实际部署步骤（停进程/复制/预构建词典/授权/注册/自启动/验证）统一由共享模块执行，
# 与 NSIS 安装器走同一条代码路径；本脚本只负责构建与产物校验。
$deployArgs = @{
    Configuration = $Configuration
    TargetDir = $targetDir
}
if (-not $NoStartHost) {
    $deployArgs['StartHost'] = $true
}
& (Join-Path $sourceRoot 'installer\Deploy-Shurufa.ps1') @deployArgs
