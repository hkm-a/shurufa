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
$profile = $Configuration.ToLowerInvariant()
$targetDir = Join-Path $sourceRoot "target\$profile"
$destination = Join-Path $env:ProgramData 'shurufa'
$librime = Join-Path $sourceRoot 'third_party\librime\dist'

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
    (Join-Path $targetDir 'shurufa-host.exe'),
    (Join-Path $targetDir 'Shurufa.exe'),
    (Join-Path $librime 'lib\rime.dll'),
    (Join-Path $librime 'bin\rime_deployer.exe'),
    (Join-Path $sourceRoot 'installer\register-host-startup.ps1')
)
foreach ($file in $required) {
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) {
        throw "缺少安装文件：$file"
    }
}

if (Test-Path -LiteralPath (Join-Path $destination 'shurufa-host.exe') -PathType Leaf) {
    & (Join-Path $destination 'shurufa-host.exe') stop
}
Get-Process -Name ctfmon, TextInputHost -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
$oldDll = Join-Path $destination 'shurufa_tsf.dll'
if (Test-Path -LiteralPath $oldDll -PathType Leaf) {
    & regsvr32.exe /u /s $oldDll
}
Start-Sleep -Seconds 1

try {
    New-Item -ItemType Directory -Path $destination -Force | Out-Null
    Copy-Item (Join-Path $targetDir 'shurufa_tsf.dll') $destination -Force
    Copy-Item (Join-Path $targetDir 'shurufa-algo.exe') $destination -Force
    Copy-Item (Join-Path $targetDir 'shurufa-host.exe') $destination -Force
    Copy-Item (Join-Path $targetDir 'Shurufa.exe') $destination -Force
    Copy-Item (Join-Path $librime 'lib\rime.dll') $destination -Force
    Copy-Item (Join-Path $librime 'bin\rime_deployer.exe') $destination -Force
    Copy-Item (Join-Path $sourceRoot 'installer\register-host-startup.ps1') $destination -Force
    Copy-Item (Join-Path $sourceRoot 'schemas') (Join-Path $destination 'schemas') -Recurse -Force

    $deployer = Join-Path $destination 'rime_deployer.exe'
    $schemas = Join-Path $destination 'schemas'
    Invoke-Native $deployer @('--build', $schemas, $schemas, (Join-Path $schemas 'build'))

    Invoke-Native 'icacls.exe' @($destination, '/grant', '*S-1-15-2-1:(OI)(CI)(RX)', '/t', '/c')
    Invoke-Native 'regsvr32.exe' @('/s', (Join-Path $destination 'shurufa_tsf.dll'))
    Set-WinDefaultInputMethodOverride -InputTip '0804:{8A5C1B49-3D2E-4F7A-9C61-0B7E2D5A9F13}{C4E9D2A7-6B31-4A58-8F0D-1E9A7C3B5D26}'
    $startupState = Join-Path $destination '.startup-state.json'
    & (Join-Path $destination 'register-host-startup.ps1') -InstallDir $destination -StatePath $startupState
    Remove-Item -LiteralPath $startupState -Force
}
catch {
    throw
}

if (-not $NoStartHost) {
    Start-Process -FilePath (Join-Path $destination 'shurufa-host.exe') -ArgumentList 'supervise' -WindowStyle Hidden
}

Write-Host "Shurufa 已安装到 $destination。"
Write-Host 'Shurufa 拼音已设为默认输入法，后台服务会在后续登录时自动启动。'
if ($NoStartHost) {
    Write-Host "可随后运行：& '$destination\shurufa-host.exe' supervise"
}
