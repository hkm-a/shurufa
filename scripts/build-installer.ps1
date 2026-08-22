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
$manifest = Get-Content -LiteralPath (Join-Path $sourceRoot 'version.json') -Raw | ConvertFrom-Json
$version = [string]$manifest.version

# 打包前强制校验版本一致性（FOX 安装器 tauri.conf.json 与 version.json 对齐）。
& (Join-Path $sourceRoot 'scripts\set-version.ps1') -Check
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# 先重新生成三份构建期产物，避免仓内生成物过期（阶段 3）。
& (Join-Path $sourceRoot 'scripts\regenerate-generated.ps1')
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

if (-not $SkipBuild) {
    Push-Location $sourceRoot
    try {
        # 运行时依赖（安装引擎 payload 与设置页）
        Invoke-Native 'cargo' @('build', '--release', '-p', 'shurufa-tsf', '-p', 'shurufa-algo', '-p', 'shurufa-host')
        Invoke-Native 'npm' @('--prefix', 'platforms/windows-settings', 'run', 'tauri', '--', 'build', '--no-bundle')
        # FOX 安装器本体（Tauri 自定义外壳，payload 由 build.rs 嵌入）
        Invoke-Native 'cargo' @('build', '--release', '-p', 'shurufa-installer')
    }
    finally {
        Pop-Location
    }
}

$output = Join-Path $sourceRoot "target\release\FOX-Setup.exe"
if (-not (Test-Path -LiteralPath $output -PathType Leaf)) {
    throw "安装包未生成：$output"
}

New-Item -ItemType Directory -Path (Join-Path $sourceRoot 'dist') -Force | Out-Null
$dist = Join-Path $sourceRoot "dist\FOX-Setup-$version.exe"
Copy-Item -LiteralPath $output -Destination $dist -Force

$checksumFile = "$dist.sha256"
Get-FileHash -LiteralPath $dist -Algorithm SHA256 |
    ForEach-Object { '{0}  {1}' -f $_.Hash.ToLowerInvariant(), (Split-Path -Leaf $dist) } |
    Set-Content -LiteralPath $checksumFile -Encoding ascii
Write-Host "SHA256 已写入：$checksumFile"

Write-Host "安装包已生成：$dist"
