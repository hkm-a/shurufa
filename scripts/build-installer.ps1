[CmdletBinding()]
param(
    [switch]$SkipBuild,

    # 配置 SHURUFA_SIGN_PFX / SHURUFA_SIGN_PASSWORD 环境变量并提供 -Sign 时签名；
    # 未配置证书时打印醒目警告但仍然产出未签名安装包，方便本地验证。
    [switch]$Sign
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

# 打包前强制校验四个派生点的版本一致性，避免发布 Windows/Android 不同步的包。
& (Join-Path $sourceRoot 'scripts\set-version.ps1') -Check
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$makensis = Join-Path ${env:ProgramFiles(x86)} 'NSIS\makensis.exe'
if (-not (Test-Path -LiteralPath $makensis -PathType Leaf)) {
    throw "未找到 NSIS 编译器：$makensis"
}

if (-not $SkipBuild) {
    Push-Location $sourceRoot
    try {
        Invoke-Native 'cargo' @('build', '-p', 'shurufa-tsf', '-p', 'shurufa-algo', '-p', 'shurufa-host', '--release')
        Invoke-Native 'npm' @('--prefix', 'platforms/windows-settings', 'run', 'tauri', '--', 'build', '--no-bundle')
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
$output = Join-Path $sourceRoot "dist\Shurufa-Setup-$version.exe"
if (-not (Test-Path -LiteralPath $output -PathType Leaf)) {
    throw "安装包未生成：$output"
}

$checksumFile = "$output.sha256"
Get-FileHash -LiteralPath $output -Algorithm SHA256 |
    ForEach-Object { '{0}  {1}' -f $_.Hash.ToLowerInvariant(), (Split-Path -Leaf $output) } |
    Set-Content -LiteralPath $checksumFile -Encoding ascii
Write-Host "SHA256 已写入：$checksumFile"

function Invoke-SignFile([string]$File) {
    $pfx = $env:SHURUFA_SIGN_PFX
    $password = $env:SHURUFA_SIGN_PASSWORD
    if ([string]::IsNullOrWhiteSpace($pfx) -or -not (Test-Path -LiteralPath $pfx -PathType Leaf)) {
        Write-Warning "未配置代码签名证书（SHURUFA_SIGN_PFX），安装包与二进制将保持未签名状态。`n用户首次运行未签名的输入法安装器会被 SmartScreen / 杀软拦截。`n正式分发前请配置证书后重新运行：scripts\build-installer.ps1 -Sign"
        return
    }
    $signtool = Get-ChildItem -Path "${env:ProgramFiles(x86)}\Windows Kits\10\bin" -Recurse -Filter 'signtool.exe' -ErrorAction SilentlyContinue |
        Sort-Object FullName -Descending | Select-Object -First 1
    if (-not $signtool) {
        throw "指定了 -Sign 但未找到 signtool.exe，请安装 Windows SDK 签名工具。"
    }
    $args = @('sign', '/fd', 'sha256', '/td', 'sha256', '/tr', 'http://timestamp.digicert.com',
              '/f', $pfx, '/p', $password, $File)
    Invoke-Native $signtool.FullName $args
    Write-Host "已签名：$File"
}

$payload = @(
    (Join-Path $sourceRoot 'target\release\shurufa_tsf.dll'),
    (Join-Path $sourceRoot 'target\release\shurufa-algo.exe'),
    (Join-Path $sourceRoot 'target\release\shurufa-host.exe'),
    (Join-Path $sourceRoot 'target\release\Shurufa.exe'),
    $output
)
foreach ($file in $payload) {
    Invoke-SignFile $file
}

Write-Host "安装包已生成：$output"
