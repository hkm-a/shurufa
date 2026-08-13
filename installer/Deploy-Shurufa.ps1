#Requires -RunAsAdministrator
[CmdletBinding()]
param(
    # 部署目标目录；默认与安装器一致（SetShellVarContext all 后的 $APPDATA 即 ProgramData）。
    [string]$InstallDir = (Join-Path $env:ProgramData 'shurufa'),

    # 构建配置。非 Mandatory：本文件会被 register-host-startup/activate-default-ime/verify-install
    # 以点源方式复用（不带参数），若 Mandatory 会在非交互 PowerShell 里报错导致安装流程中止。
    [ValidateSet('Debug', 'Release')]
    [string]$Configuration = 'Release',

    # 本轮构建产物所在目录（默认 target\<Configuration>）。
    [string]$TargetDir,

    # 部署完成后启动后台服务并做终态验证。
    [switch]$StartHost
)

$ErrorActionPreference = 'Stop'

# InputTip/自启动注册表名只在本文件定义；activate-default-ime.ps1、
# register-host-startup.ps1、verify-install.ps1 均 dot-source 本文件复用，GUID 全仓唯一来源。
$script:ShurufaInputTip = '0804:{8A5C1B49-3D2E-4F7A-9C61-0B7E2D5A9F13}{C4E9D2A7-6B31-4A58-8F0D-1E9A7C3B5D26}'

function Invoke-Native([string]$File, [string[]]$Arguments) {
    & $File @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "命令失败（退出码 $LASTEXITCODE）：$File $($Arguments -join ' ')"
    }
}

function Resolve-ShurufaPaths {
    param(
        [Parameter(Mandatory)][string]$Dir,
        [string]$Configuration = 'Release',
        [string]$TargetDir
    )
    $sourceRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
    if (-not $TargetDir) {
        $TargetDir = Join-Path $sourceRoot ('target\' + $Configuration.ToLowerInvariant())
    }
    $version = (Get-Content -LiteralPath (Join-Path $sourceRoot 'version.json') -Raw |
        ConvertFrom-Json).version
    return [pscustomobject]@{
        InstallDir = $Dir
        TargetDir = $TargetDir
        LibrimeDist = Join-Path $sourceRoot 'third_party\librime\dist'
        SourceRoot = $sourceRoot
        TsfVersionedName = "shurufa_tsf-$version.dll"
    }
}

function Resolve-ShurufaHostSuperviseCommand {
    param([Parameter(Mandatory)][string]$Dir)
    return '"{0}" supervise' -f (Join-Path $Dir 'shurufa-host.exe')
}

function Get-ShurufaStartupEntry {
    $entry = Get-ItemProperty -LiteralPath 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' `
        -Name 'ShurufaHost' -ErrorAction SilentlyContinue
    if ($entry) { return $entry.PSObject.Properties['ShurufaHost'].Value }
    return $null
}

function Register-ShurufaStartup {
    param([Parameter(Mandatory)][string]$Dir)
    $expected = Resolve-ShurufaHostSuperviseCommand -Dir $Dir
    $hostPath = Join-Path $Dir 'shurufa-host.exe'
    if (-not (Test-Path -LiteralPath $hostPath -PathType Leaf)) {
        throw "后台宿主不存在：$hostPath"
    }
    $runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
    New-Item -Path $runKey -Force | Out-Null
    New-ItemProperty -LiteralPath $runKey -Name 'ShurufaHost' -Value $expected -PropertyType String -Force | Out-Null
    $actual = Get-ShurufaStartupEntry
    if ($actual -ne $expected) {
        throw "登录自启动项写入后读回不一致：期望 [$expected]，实际 [$actual]"
    }
}

function Unregister-ShurufaStartup {
    Remove-ItemProperty -LiteralPath 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' `
        -Name 'ShurufaHost' -ErrorAction SilentlyContinue
}

function Set-ShurufaDefaultIme {
    Set-WinDefaultInputMethodOverride -InputTip $script:ShurufaInputTip
    if ((Get-WinDefaultInputMethodOverride).InputMethodTip -ne $script:ShurufaInputTip) {
        throw 'Windows 未确认 Shurufa 拼音已成为默认输入法。'
    }
}

function Clear-ShurufaDefaultIme {
    # 仅在默认覆盖仍指向 Shurufa 时清除，避免动到用户后来改选的其他输入法。
    if ((Get-WinDefaultInputMethodOverride).InputMethodTip -eq $script:ShurufaInputTip) {
        Set-WinDefaultInputMethodOverride
    }
}

function Test-ShurufaInstallComplete {
    param(
        [Parameter(Mandatory)][string]$Dir,
        [int]$TimeoutSeconds = 12
    )
    $expected = Resolve-ShurufaHostSuperviseCommand -Dir $Dir
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ($true) {
        $problems = @()
        if ((Get-ShurufaStartupEntry) -ne $expected) {
            $problems += "登录自启动项异常：期望 [$expected]，实际 [$(Get-ShurufaStartupEntry)]"
        }
        if (-not (Get-Process -Name 'shurufa-host' -ErrorAction SilentlyContinue)) {
            $problems += '后台宿主 shurufa-host.exe 未运行'
        }
        if (-not (Get-Process -Name 'shurufa-algo' -ErrorAction SilentlyContinue)) {
            $problems += '算法服务 shurufa-algo.exe 未运行'
        }
        if (-not $problems) {
            Write-Output '安装验证通过：自启动项与后台进程均就绪。'
            return $true
        }
        if ((Get-Date) -ge $deadline) { break }
        Start-Sleep -Milliseconds 500
    }
    $problems | ForEach-Object { Write-Output "验证失败：$_" }
    return $false
}

function Stop-ShurufaInstall {
    param([Parameter(Mandatory)][string]$Dir)
    # 覆盖安装前释放所有可能锁住 DLL/EXE 的进程；未运行时静默继续。
    Get-Process -Name 'Shurufa', 'ctfmon', 'TextInputHost' -ErrorAction SilentlyContinue |
        Stop-Process -Force -ErrorAction SilentlyContinue
    $hostExe = Join-Path $Dir 'shurufa-host.exe'
    if (Test-Path -LiteralPath $hostExe -PathType Leaf) {
        & $hostExe stop | Out-Null
    }
    Start-Sleep -Seconds 1
}

function Unregister-OldTsf {
    param(
        [Parameter(Mandatory)][string]$Dir,
        [Parameter(Mandatory)][string]$VersionedName
    )
    foreach ($name in @($VersionedName, 'shurufa_tsf.dll')) {
        $dll = Join-Path $Dir $name
        if (Test-Path -LiteralPath $dll -PathType Leaf) {
            Invoke-Native 'regsvr32.exe' @('/u', '/s', $dll)
        }
    }
    Start-Sleep -Seconds 1
}

function Install-ShurufaFiles {
    param([Parameter(Mandatory)]$Paths)
    New-Item -ItemType Directory -Path $Paths.InstallDir -Force | Out-Null
    $versionedTarget = Join-Path $Paths.InstallDir $Paths.TsfVersionedName
    # 版本化 DLL 已存在时不再覆盖（Windows 可能仍加载旧副本），其余文件总是原位更新。
    if (-not (Test-Path -LiteralPath $versionedTarget -PathType Leaf)) {
        Copy-Item (Join-Path $Paths.TargetDir 'shurufa_tsf.dll') $versionedTarget
    }
    foreach ($exe in @('shurufa-algo.exe', 'shurufa-host.exe', 'Shurufa.exe')) {
        Copy-Item (Join-Path $Paths.TargetDir $exe) $Paths.InstallDir -Force
    }
    Copy-Item (Join-Path $Paths.LibrimeDist 'lib\rime.dll') $Paths.InstallDir -Force
    Copy-Item (Join-Path $Paths.LibrimeDist 'bin\rime_deployer.exe') $Paths.InstallDir -Force
    foreach ($script in @('activate-default-ime.ps1', 'register-host-startup.ps1', 'verify-install.ps1', 'Deploy-Shurufa.ps1')) {
        Copy-Item (Join-Path $Paths.SourceRoot "installer\$script") $Paths.InstallDir -Force
    }
    Copy-Item (Join-Path $Paths.SourceRoot 'schemas') (Join-Path $Paths.InstallDir 'schemas') -Recurse -Force
}

function Build-ShurufaDictionaries {
    param([Parameter(Mandatory)][string]$Dir)
    $deployer = Join-Path $Dir 'rime_deployer.exe'
    $schemas = Join-Path $Dir 'schemas'
    Invoke-Native $deployer @('--build', $schemas, $schemas, (Join-Path $schemas 'build'))
}

function Grant-AppContainerAccess {
    param([Parameter(Mandatory)][string]$Dir)
    # 输入法宿主运行在 AppContainer，必须可读安装目录。
    Invoke-Native 'icacls.exe' @($Dir, '/grant', '*S-1-15-2-1:(OI)(CI)(RX)', '/t', '/c')
}

function Register-ShurufaTsf {
    param(
        [Parameter(Mandatory)][string]$Dir,
        [Parameter(Mandatory)][string]$VersionedName
    )
    Invoke-Native 'regsvr32.exe' @('/s', (Join-Path $Dir $VersionedName))
}

function Start-ShurufaHost {
    param([Parameter(Mandatory)][string]$Dir)
    Start-Process -FilePath (Join-Path $Dir 'shurufa-host.exe') -ArgumentList 'supervise' -WindowStyle Hidden
}

function Assert-ShurufaInstall {
    param([Parameter(Mandatory)][string]$Dir)
    if (-not (Test-ShurufaInstallComplete -Dir $Dir)) {
        throw '安装终态验证失败：登录自启动项或后台进程未就绪。'
    }
}

# 非 dot-source 调用时执行完整部署序列（install.ps1 / 手工部署路径）。
if ($MyInvocation.InvocationName -ne '.') {
    $paths = Resolve-ShurufaPaths -Dir $InstallDir -Configuration $Configuration -TargetDir $TargetDir
    Stop-ShurufaInstall -Dir $InstallDir
    Unregister-OldTsf -Dir $InstallDir -VersionedName $paths.TsfVersionedName
    Install-ShurufaFiles -Paths $paths
    Build-ShurufaDictionaries -Dir $InstallDir
    Grant-AppContainerAccess -Dir $InstallDir
    Register-ShurufaTsf -Dir $InstallDir -VersionedName $paths.TsfVersionedName
    Set-ShurufaDefaultIme
    Register-ShurufaStartup -Dir $InstallDir
    if ($StartHost) {
        Start-ShurufaHost -Dir $InstallDir
        Assert-ShurufaInstall -Dir $InstallDir
    }
    Write-Output "Shurufa 已部署到 $InstallDir。"
}
