# start-control-center.ps1 — 修复"悬浮球白屏"并拉起已部署的控制中心
#
# 背景（2026-08-12 两次事故 + 2026-08-19 复现）：target\debug\Shurufa.exe 是
# dev 构建，devUrl 指向 http://localhost:1420（vite）。直接双击/拷贝运行 dev 版
# 时，若 vite 未启动，WebView2 加载失败 → 白屏 + 滚动条，悬浮球消失。
# 本脚本：1) 杀掉所有残留 Shurufa.exe（dev 或旧版）；2) 用已部署的
# C:\ProgramData\shurufa\Shurufa.exe（release，内嵌 ui-dist，可独立运行）拉起；
# 3) 可选 -Deploy：先从 target/release 复制新产物再启动。
#
# 用法（仓库根目录 PowerShell）：
#   .\scripts\start-control-center.ps1            # 只拉起已部署版
#   .\scripts\start-control-center.ps1 -Deploy     # 先部署 release 再拉起
#   .\scripts\start-control-center.ps1 -Deploy -NoStart   # 只部署不启动

param(
  [switch]$Deploy,
  [switch]$NoStart
)

$ErrorActionPreference = 'Continue'
$repo = 'C:\Users\hkm\Documents\shurufa'
$dst  = 'C:\ProgramData\shurufa'
$dstExe = Join-Path $dst 'Shurufa.exe'

function Log($m) { Write-Host "[$(Get-Date -Format 'HH:mm:ss')] $m" }

# 1) 停旧：任何 Shurufa.exe（dev 白屏实例/旧部署）先杀，避免锁文件与双实例
Log '停旧：Shurufa.exe …'
Get-Process -Name Shurufa -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 800

# 2) 可选：从 release 产物部署（提权复制，与 update-all.ps1 -Cc 同款）
if ($Deploy) {
  $src = "$repo\target\release\Shurufa.exe"
  if (-not (Test-Path $src)) {
    Log "FAIL: 找不到 release 产物 $src（请先运行 .\scripts\update-all.ps1 -Cc）"
    exit 1
  }
  $elev = Join-Path $env:TEMP 'shurufa-cc-deploy.ps1'
  $elevBody = @'
$ErrorActionPreference = "Stop"
$fail = 0
$src = 'C:UsershkmDocumentsshurufa	argeteleaseShurufa.exe'
$dst = 'C:ProgramDatashurufa'
$pf  = 'C:Program FilesFOXShurufa.exe'
if (Test-Path (Join-Path $dst 'Shurufa.exe')) {
  Copy-Item (Join-Path $dst 'Shurufa.exe') (Join-Path $dst ('Shurufa.exe.bak-' + (Get-Date -Format HHmmss))) -Force -ErrorAction SilentlyContinue
}
try { Copy-Item $src (Join-Path $dst 'Shurufa.exe') -Force -ErrorAction Stop; Write-Host 'OK ProgramData' }
catch { Write-Host ('FAIL ProgramData: ' + $_.Exception.Message); $fail = 1 }
if (Test-Path $pf) {
  Copy-Item $pf ('C:Program FilesFOXShurufa.exe.bak-' + (Get-Date -Format HHmmss)) -Force -ErrorAction SilentlyContinue
}
try { Copy-Item $src $pf -Force -ErrorAction Stop; Write-Host 'OK ProgramFiles' }
catch { Write-Host ('WARN ProgramFiles: ' + $_.Exception.Message) }
exit $fail
'@
  $elevBody | Out-File $elev -Encoding UTF8
  Log '提权部署 release Shurufa.exe → ProgramData + Program Files（请在弹出的 UAC 里点"是"）…'
  $proc = Start-Process powershell -Verb RunAs -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File',$elev -Wait -PassThru
  Remove-Item $elev -Force -ErrorAction SilentlyContinue
  if ($proc.ExitCode -ne 0) { Log 'FAIL: 部署失败（提权复制报错）'; exit 1 }
  Log '部署完成'
}

if ($NoStart) { Log '完成（-NoStart，未启动）'; exit 0 }

# 3) 拉起已部署版（release 内嵌前端，无需 vite）
if (-not (Test-Path $dstExe)) {
  Log "FAIL: 未部署控制中心 $dstExe（请先 .\scripts\update-all.ps1 -Cc 或本脚本 -Deploy）"
  exit 1
}
Log "启动：$dstExe"
Start-Process -FilePath $dstExe
Start-Sleep -Seconds 4
$p = Get-Process -Name Shurufa -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $p) { Log 'FAIL: Shurufa.exe 未起来'; exit 2 }
$v = $p.Path
Log "OK：Shurufa.exe 运行中（$v）"
Log '若仍白屏：检查该进程是否来自 target\debug（本脚本已杀掉 dev 实例，正常不会）'
exit 0

