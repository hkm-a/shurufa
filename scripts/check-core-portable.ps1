# core/ 平台中立性门禁。
#
# 规则：core/ 下的 crate 是「跨平台核心」，Windows/Android 双端共用。因此它们
# 不得**无条件**依赖平台专属 crate（windows / windows-sys / windows-core /
# winapi）——平台相关依赖必须放在 [target.'cfg(windows)'.dependencies] 之类的
# 条件段里，否则该 crate 只能在 Windows 编译，Android 侧就拿不到，只能重写一份。
#
# 为什么要这条门禁：分层的失效点从来不在「有人写错了」，而在「写错了没人拦」。
# core/ime-ipc 就是这样从「IPC 协议」长成了持有全局中英状态机、打字统计埋点、
# 超长组合策略的 Windows 专属 crate，导致 Android 只能在 rimejni 里重写一份更
# 弱的按键路径，同一个 AI 跳过逻辑的 bug 只修了 Windows 一端。
#
# 实现说明：这里做的是 Cargo.toml 静态检查而非真正的 `cargo check --target
# <非 Windows>`。后者更彻底但需要在 CI 上装交叉 target 与链接器；静态检查零成本、
# 秒级完成，且足以拦住「随手加一个 windows 依赖」这个真实的退化路径。

[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$sourceRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$coreDir = Join-Path $sourceRoot 'core'

# 平台专属依赖名（出现在无条件 [dependencies] 段即违规）
$platformCrates = @('windows', 'windows-sys', 'windows-core', 'windows-registry', 'winapi', 'jni', 'ndk')

# 既有违规基线。**只允许缩短，不允许加长。**
# 每一项都必须写明欠债内容与偿还方向，删除一项即代表该债已还清。
$baseline = @{
    # ime-ipc 同时是「命名管道传输」与「输入法策略层」。策略部分（全局中英
    # 状态机、超长组合防护、打字统计埋点、候选装饰钩子）本应下沉到平台无关的
    # core/ime-policy，传输部分则应移到 platforms/windows-ipc。
    # 偿还前，Android 无法复用任何按键策略，双端行为会持续分叉。
    'ime-ipc' = 'IPC 传输与 IME 策略混装；待拆为 core/ime-policy + platforms/windows-ipc'
}

$violations = @()
$baselineHit = @()

Get-ChildItem -LiteralPath $coreDir -Directory | ForEach-Object {
    $manifest = Join-Path $_.FullName 'Cargo.toml'
    if (-not (Test-Path -LiteralPath $manifest)) { return }

    $crateName = $_.Name
    $lines = Get-Content -LiteralPath $manifest
    $section = ''
    foreach ($line in $lines) {
        $trimmed = $line.Trim()
        if ($trimmed -match '^\[(.+)\]$') { $section = $Matches[1]; continue }
        if ($trimmed -match '^#') { continue }
        # 只看无条件依赖段；[target.'cfg(...)'.dependencies] 是允许的
        if ($section -notin @('dependencies', 'build-dependencies')) { continue }
        if ($trimmed -match '^([A-Za-z0-9_-]+)\s*(=|\.)') {
            $dep = $Matches[1]
            if ($platformCrates -contains $dep) {
                if ($baseline.ContainsKey($crateName)) {
                    $baselineHit += "$crateName -> $dep"
                }
                else {
                    $violations += "core/$crateName 的 [$section] 无条件依赖 $dep"
                }
            }
        }
    }
}

foreach ($hit in ($baselineHit | Sort-Object -Unique)) {
    Write-Host "[基线欠债] $hit"
}
foreach ($k in $baseline.Keys) {
    Write-Host "           $k : $($baseline[$k])"
}

if ($violations.Count -gt 0) {
    Write-Host ''
    foreach ($v in $violations) { Write-Host "[违规] $v" }
    throw @"
core/ 平台中立性检查未通过。

core/ 下的 crate 必须能被 Windows 与 Android 双端共用，平台专属依赖请改放到条件段：

    [target.'cfg(windows)'.dependencies]
    windows-sys = { version = "...", features = [...] }

若这段逻辑本身就是平台专属的，它不该住在 core/——请移到 platforms/ 下。
参考 core/sync/Cargo.toml 的正确写法。
"@
}

Write-Host ''
Write-Host "core/ 平台中立性检查通过（基线欠债 $($baseline.Count) 项，只允许减少）。"
exit 0
