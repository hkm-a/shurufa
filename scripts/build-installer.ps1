[CmdletBinding()]
param(
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'

function Invoke-Native([string]$File, [string[]]$Arguments) {
    & $File @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "命令失败（退出码 $LASTEXITCODE）：$File $($Arguments -join ' ')"
    }
}

$sourceRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$makensis = Join-Path ${env:ProgramFiles(x86)} 'NSIS\makensis.exe'
if (-not (Test-Path -LiteralPath $makensis -PathType Leaf)) {
    throw "未找到 NSIS 编译器：$makensis"
}

if (-not $SkipBuild) {
    Push-Location $sourceRoot
    try {
        Invoke-Native 'cargo' @('build', '-p', 'shurufa-tsf', '-p', 'shurufa-algo', '-p', 'shurufa-host', '-p', 'shurufa-settings', '--release')
    }
    finally {
        Pop-Location
    }
}

$required = @(
    (Join-Path $sourceRoot 'target\release\shurufa_tsf.dll'),
    (Join-Path $sourceRoot 'target\release\shurufa-algo.exe'),
    (Join-Path $sourceRoot 'target\release\shurufa-host.exe'),
    (Join-Path $sourceRoot 'target\release\Shurufa.exe'),
    (Join-Path $sourceRoot 'third_party\librime\dist\lib\rime.dll'),
    (Join-Path $sourceRoot 'third_party\librime\dist\bin\rime_deployer.exe')
)
foreach ($file in $required) {
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) {
        throw "缺少安装文件：$file"
    }
}

New-Item -ItemType Directory -Path (Join-Path $sourceRoot 'dist') -Force | Out-Null
Invoke-Native $makensis @('/INPUTCHARSET', 'UTF8', (Join-Path $sourceRoot 'installer\shurufa.nsi'))
$output = Join-Path $sourceRoot 'dist\Shurufa-Setup.exe'
if (-not (Test-Path -LiteralPath $output -PathType Leaf)) {
    throw "安装包未生成：$output"
}
Write-Host "安装包已生成：$output"
