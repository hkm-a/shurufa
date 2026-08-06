[CmdletBinding()]
param(
    [string]$InstallDir,
    [switch]$Remove
)

$ErrorActionPreference = 'Stop'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$runValueName = 'shurufa-host'

if ($Remove) {
    Remove-ItemProperty -LiteralPath $runKey -Name $runValueName -ErrorAction SilentlyContinue
    exit 0
}

if ([string]::IsNullOrWhiteSpace($InstallDir)) {
    throw '安装目录不能为空。'
}

$hostPath = Join-Path $InstallDir 'shurufa-host.exe'
if (-not (Test-Path -LiteralPath $hostPath -PathType Leaf)) {
    throw "后台宿主不存在：$hostPath"
}

# 登录自启动必须指向 supervise（监管模式）：worker 与算法服务由监管进程看护，
# 崩溃自动重启；写入后必须读回核验，防止任何静默失败。
$expectedValue = '"{0}" supervise' -f $hostPath
New-Item -Path $runKey -Force | Out-Null
New-ItemProperty -LiteralPath $runKey -Name $runValueName -Value $expectedValue -PropertyType String -Force | Out-Null
$registered = (Get-ItemProperty -LiteralPath $runKey -Name $runValueName).PSObject.Properties[$runValueName].Value
if ($registered -ne $expectedValue) {
    throw 'Windows 未确认 Shurufa 后台服务的登录启动配置。'
}
