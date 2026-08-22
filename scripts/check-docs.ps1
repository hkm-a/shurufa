# 文档一致性门禁。
#
# 为什么要它：本仓库的文档曾同时存在三个互相矛盾的「当前版本」、指向未入库
# 文件的链接、以及承诺了与实现相反的行为（版本管理.md 曾把 gradle.properties
# 说成自动同步派生点，照做会直接锁死 CI）。文档失真不会被任何编译器发现，
# 唯一的办法是把「文档必须成立的断言」写成可执行检查。
#
# 原则：写不进本脚本的文档承诺，就不要写进文档。
#
# 检查项：
#   1. 内部链接可达——md 里指向仓内文件的相对链接必须真实存在（含 .claude/
#      这类被 gitignore 的路径：对 clone 下来的人就是死链）。
#   2. 版本号声明——文档里写死的 vX.Y.Z「当前版本」必须等于 version.json。
#      （结构化派生点由 set-version.ps1 -Check 负责，此处只管散落在正文里的。）
#   3. CHANGELOG 对账——version.json 的当前版本必须已有 CHANGELOG 条目。

[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$sourceRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$manifest = Get-Content -LiteralPath (Join-Path $sourceRoot 'version.json') -Raw | ConvertFrom-Json
$currentVersion = [string]$manifest.version

$problems = @()

# 仅检查 git 跟踪的 md，避免扫到 target/ 与第三方目录
Push-Location $sourceRoot
try { $tracked = & git ls-files '*.md' } finally { Pop-Location }

foreach ($rel in $tracked) {
    if ([string]::IsNullOrWhiteSpace($rel)) { continue }
    $full = Join-Path $sourceRoot $rel
    if (-not (Test-Path -LiteralPath $full)) { continue }
    $text = Get-Content -LiteralPath $full -Raw
    $docDir = Split-Path -Parent $full

    # ---- 1. 内部链接可达 ----
    # 链接目标不允许跨行、不允许含空白与中文全角括号：曾有一处把 markdown
    # 链接写成 (开发计划.md） —— 全角右括号不闭合，宽松的正则会一路吞到文件
    # 末尾的下一个半角右括号，既掩盖了真实的死链，也让报错信息不可读。
    foreach ($m in [regex]::Matches($text, '\[[^\]\r\n]*\]\(([^)\r\n（）\s]+)\)')) {
        $target = $m.Groups[1].Value.Trim()
        # 跳过外链、锚点、模板占位
        if ($target -match '^(https?:|mailto:|#|<)') { continue }
        # 去掉链接内的锚点
        $path = ($target -split '#')[0]
        if ([string]::IsNullOrWhiteSpace($path)) { continue }
        # 指向 GitHub 仓库页的相对写法（../../releases 等）不是文件系统路径
        if ($path -match '^\.\./\.\./') { continue }
        $resolved = Join-Path $docDir $path
        if (-not (Test-Path -LiteralPath $resolved)) {
            $problems += "$rel 链接失效：$target"
        }
    }

    # 全角括号闭合的 markdown 链接：渲染时就是坏的，单独报出来
    foreach ($m in [regex]::Matches($text, '\[[^\]\r\n]*\]\([^)\r\n]*）')) {
        $problems += "$rel 链接用全角）闭合，渲染会失效：$($m.Value.Trim())"
    }

    # ---- 2. 版本号声明 ----
    # 只认「当前版本 vX.Y.Z」「状态-vX.Y.Z」这类明确的当前版本声明；
    # CHANGELOG 与历史叙述里的版本号不在此列。
    foreach ($m in [regex]::Matches($text, '当前版本\s*\**v(\d+\.\d+\.\d+)')) {
        if ($m.Groups[1].Value -ne $currentVersion) {
            $problems += "$rel 声明当前版本 v$($m.Groups[1].Value)，但 version.json 是 $currentVersion"
        }
    }
    foreach ($m in [regex]::Matches($text, '状态-v(\d+\.\d+\.\d+)-')) {
        if ($m.Groups[1].Value -ne $currentVersion) {
            $problems += "$rel 状态徽章为 v$($m.Groups[1].Value)，但 version.json 是 $currentVersion"
        }
    }
}

# ---- 3. CHANGELOG 对账 ----
$changelog = Get-Content -LiteralPath (Join-Path $sourceRoot 'CHANGELOG.md') -Raw
if ($changelog -notmatch "(?m)^## \[$([regex]::Escape($currentVersion))\]") {
    $problems += "CHANGELOG.md 缺少 ## [$currentVersion] 条目（version.json 已是该版本）"
}

if ($problems.Count -gt 0) {
    Write-Host "文档一致性检查未通过，共 $($problems.Count) 项：`n"
    foreach ($p in $problems) { Write-Host "  - $p" }
    throw "文档一致性检查未通过。请修正上述条目；若某条承诺无法被机器验证，就不要写进文档。"
}

Write-Host "文档一致性检查通过（当前版本 $currentVersion，已扫描 $($tracked.Count) 个 md）。"
exit 0
