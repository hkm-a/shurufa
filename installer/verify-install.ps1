[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$InstallDir,
    [int]$TimeoutSeconds = 12
)

# 安装终态验证：确认"输入法后台持续运行"所需的全部条件已就绪。
# 供安装器收尾调用，也可随时手动运行自检。退出码 0 表示全部通过。
$ErrorActionPreference = 'Stop'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$expectedValue = '"{0}" supervise' -f (Join-Path $InstallDir 'shurufa-host.exe')
$deadline = (Get-Date).AddSeconds($TimeoutSeconds)
$problems = @()

while ($true) {
    $problems = @()
    $entry = Get-ItemProperty -LiteralPath $runKey -Name 'shurufa-host' -ErrorAction SilentlyContinue
    $value = if ($entry) { $entry.PSObject.Properties['shurufa-host'].Value } else { $null }
    if ($value -ne $expectedValue) {
        $problems += "登录自启动项异常：期望 [$expectedValue]，实际 [$value]"
    }
    if (-not (Get-Process -Name 'shurufa-host' -ErrorAction SilentlyContinue)) {
        $problems += '后台宿主 shurufa-host.exe 未运行'
    }
    if (-not (Get-Process -Name 'shurufa-algo' -ErrorAction SilentlyContinue)) {
        $problems += '算法服务 shurufa-algo.exe 未运行'
    }
    if (-not $problems) {
        Write-Output '安装验证通过：自启动项与后台进程均就绪。'
        exit 0
    }
    if ((Get-Date) -ge $deadline) {
        break
    }
    Start-Sleep -Milliseconds 500
}

$problems | ForEach-Object { Write-Output "验证失败：$_" }
exit 1
