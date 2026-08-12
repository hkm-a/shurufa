# update-all.ps1 — Shurufa 桌面端一键更新（固定部署程序）
#
# 背景：历史上部署脚本各自为战（deploy-no-start / update-v4 / deploy-cc /
# deploy-tsf-versioned / deploy-schemas），互相覆盖且踩坑：
#   - 从 target/debug 复制 Shurufa.exe → dev 模式白屏/无响应（2026-08-12 两次事故）
#   - 提权拉起 supervisor → algo 管道 SDDL 拒绝普通用户（os error -2147024891）
#   - supervisor 不重启、无验证环节
# 本脚本把整条链路固化为"一套固定程序"：构建 → 停旧 → 提权复制 → 普通用户
# 重启 → 健康验证（进程 + daemon.state + IPC 探针延迟）。
#
# 用法（仓库根目录执行，PowerShell）：
#   .\scripts\update-all.ps1                 # 默认：更新服务二进制（host+algo）
#   .\scripts\update-all.ps1 -Cc             # 服务 + 控制中心（tauri build，较慢）
#   .\scripts\update-all.ps1 -Tsf            # 服务 + TSF DLL（需注销重进生效）
#   .\scripts\update-all.ps1 -Schemas        # 服务 + schemas 同步
#   .\scripts\update-all.ps1 -All            # 全部（服务+控制中心+TSF+schemas）
#   .\scripts\update-all.ps1 -Services -NoBuild -NoStart   # 只用现有产物部署，不启动
#
# 退出码：0 = 成功；非 0 = 失败（验证不过）。

param(
  [switch]$Services,   # 构建+部署服务二进制（shurufa-host.exe / shurufa-algo.exe）
  [switch]$Cc,         # 构建+部署控制中心（npm run tauri build → release Shurufa.exe）
  [switch]$Tsf,        # 构建+部署 TSF DLL（版本化文件名，需注销/重启应用生效）
  [switch]$Schemas,    # robocopy /MIR 同步 schemas 到 ProgramData
  [switch]$All,        # 等价 -Services -Cc -Tsf -Schemas
  [switch]$NoBuild,    # 跳过构建，只用现有产物
  [switch]$NoStart     # 只部署，不启动 supervisor（供批量/无人值守用）
)

$ErrorActionPreference = 'Continue'
$repo = 'C:\Users\hkm\Documents\shurufa'
$dst  = 'C:\ProgramData\shurufa'
$log  = "$repo\dist\staging\update-all.log"
function Log($m) { $line = "[$(Get-Date -Format 'HH:mm:ss')] $m"; Write-Host $line; $line | Out-File $log -Append }

# 默认行为：任何组件开关都没给时 = 只更新服务
if (-not ($Services -or $Cc -or $Tsf -or $Schemas -or $All)) { $Services = $true }
if ($All) { $Services = $true; $Cc = $true; $Tsf = $true; $Schemas = $true }

"=== $(Get-Date -Format o) ===" | Out-File $log
Log "开始更新：Services=$Services Cc=$Cc Tsf=$Tsf Schemas=$Schemas Build=$(-not $NoBuild) Start=$(-not $NoStart)"

# ---------- 1) 构建（普通用户即可） ----------
if (-not $NoBuild) {
  if ($Services) {
    Log "构建服务：cargo build -p shurufa-host -p shurufa-algo …"
    Push-Location $repo
    cargo build -p shurufa-host -p shurufa-algo 2>&1 | Select-String 'error|Finished' | ForEach-Object { Log $_ }
    Pop-Location
  }
  if ($Tsf) {
    Log "构建 TSF：cargo build -p shurufa-tsf …"
    Push-Location $repo
    cargo build -p shurufa-tsf 2>&1 | Select-String 'error|Finished' | ForEach-Object { Log $_ }
    Pop-Location
  }
  if ($Cc) {
    Log "构建控制中心：npm run tauri build（务必 release，勿用裸 cargo build）…"
    Push-Location "$repo\platforms\windows-settings"
    npm run tauri build 2>&1 | Select-String 'error|Finished|release' | ForEach-Object { Log $_ }
    Pop-Location
    if (-not (Test-Path "$repo\target\release\Shurufa.exe")) {
      Log "FAIL: tauri build 未产出 target/release/Shurufa.exe"
      exit 1
    }
  }
} else {
  Log "跳过构建（-NoBuild）"
}

# ---------- 2) 组装提权脚本：停旧 → 复制（一次 UAC 完成，不拉进程） ----------
# 停旧必须在复制之前：运行中的 host/algo 会锁住 exe 导致 Copy-Item 失败，
# 且新 supervisor 抢不到单实例锁会直接退出（"更新了个寂寞"）。
$copyLines = @()
if ($Services) {
  foreach ($f in 'shurufa-host.exe','shurufa-algo.exe') {
    $s = "$repo\target\debug\$f"
    if (Test-Path $s) { $copyLines += "Copy-Item '$s' '$dst\$f' -Force -ErrorAction Stop" }
    else { Log "WARN: 产物缺失 $s"; }
  }
}
if ($Cc) {
  # 控制中心只从 release 部署——target/debug 的是 dev 模式（白屏）
  $s = "$repo\target\release\Shurufa.exe"
  if (Test-Path $s) { $copyLines += "Copy-Item '$s' '$dst\Shurufa.exe' -Force -ErrorAction Stop" }
}
if ($Schemas) {
  # robocopy /MIR 同步 schemas（排除 build/ 中间产物）
  $copyLines += "robocopy '$repo\schemas' '$dst\schemas' /MIR /XD build /NFL /NDL /NJH /NJS /NP | Out-Null"
}
if ($Tsf) {
  # TSF DLL 版本化 rename-trick：旧 DLL 被 explorer 锁定时先改名再复制
  $copyLines += @'
$ver = Get-ChildItem "$dst" -Filter 'shurufa_tsf-*.dll' | Sort-Object LastWriteTime -Descending | Select-Object -First 1
$name = if ($ver) { $ver.Name } else { 'shurufa_tsf-0.4.1.dll' }
$tsfOld = "$dst\$name"
if (Test-Path $tsfOld) { Rename-Item $tsfOld "$name.old-$(Get-Date -Format HHmmss)" -Force -ErrorAction SilentlyContinue }
Copy-Item "$repo\target\debug\shurufa_tsf.dll" $tsfOld -Force -ErrorAction Stop
'@
}

if ($copyLines.Count -eq 0) {
  Log "没有可复制的产物（检查 -NoBuild 时产物是否已构建）"
  exit 1
}

# 停旧：先普通用户发 stop-token（supervisor 自停），提权兜底强杀残留。
# 直接提权强杀有误杀 explorer 风险，故优先 stop-token 受控停机。
& "$dst\shurufa-host.exe" stop 2>&1 | Out-Null
Start-Sleep -Milliseconds 1200

$elev = "$env:TEMP\shurufa-update-copy.ps1"
$elevBody = @(
  '$ErrorActionPreference = "Stop"'
  'foreach ($n in "shurufa-host","shurufa-algo","shurufa-host-worker") {'
  '  Get-Process -Name $n -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue'
  '}'
  'Start-Sleep -Milliseconds 500'
  '$fail = 0'
)
foreach ($line in $copyLines) {
  $elevBody += "try { $line; Write-Host `"OK  $line`" } catch { Write-Host `"FAIL $line : `$_`"; `$fail = 1 }"
}
$elevBody += 'exit $fail'
$elevBody | Out-File $elev -Encoding UTF8
Log "提权：停旧 + 复制 $($copyLines.Count) 项到 ProgramData …"
$proc = Start-Process powershell -Verb RunAs -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File',"`"$elev`"" -Wait -PassThru
Remove-Item $elev -Force -ErrorAction SilentlyContinue
if ($proc.ExitCode -ne 0) {
  Log "FAIL: 提权复制失败（exit=$($proc.ExitCode)）—— 产物被占用或源缺失？"
  exit 1
}
Log "复制完成，等待 1s 让文件句柄释放…"
Start-Sleep -Seconds 1

# ---------- 3) 启动 supervisor（必须普通用户，否则 algo 管道 SDDL 拒绝访问） ----------
if ($NoStart) {
  Log "完成（-NoStart，未启动 supervisor）"
  exit 0
}
Log "以普通用户启动 supervisor …"
Start-Process -FilePath "$dst\shurufa-host.exe" -ArgumentList 'supervise' -WindowStyle Hidden

# ---------- 4) 健康验证 ----------
Log "等待服务就绪（algo 引擎首次部署可能需数十秒）…"
$ready = $false
for ($i = 0; $i -lt 40; $i++) {
  Start-Sleep -Seconds 1
  $a = Get-Process shurufa-algo -ErrorAction SilentlyContinue
  $h = Get-Process shurufa-host -ErrorAction SilentlyContinue
  if ($a -and ($h | Measure-Object).Count -ge 1) { $ready = $true; break }
}
if (-not $ready) {
  Log "FAIL: supervisor/algo 未就绪（进程缺失）"
  exit 2
}
Log "进程就绪：algo pid=$($a.Id) host=$((($h | ForEach-Object Id) -join ','))"

$state = "$env:APPDATA\shurufa\daemon.state"
if (Test-Path $state) {
  Get-Content $state | ForEach-Object { Log "  state> $_" }
}

Log "IPC 健康探针（ToggleAscii 延迟）…"
$probe = & python "$repo\scripts\verify-ipc.py" 2>&1
$probe | ForEach-Object { Log "  probe> $_" }
if ($LASTEXITCODE -ne 0) {
  Log "FAIL: IPC 探针未通过（exit=$LASTEXITCODE）"
  exit 3
}

if ($Cc) {
  $f = Get-Item "$dst\Shurufa.exe" -ErrorAction SilentlyContinue
  if ($f) { Log "控制中心：$($f.Length) bytes（release 应为 $((Get-Item "$repo\target\release\Shurufa.exe").Length)）" }
}
Log "=== 更新完成 ==="
exit 0
