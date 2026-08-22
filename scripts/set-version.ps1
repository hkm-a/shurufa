[CmdletBinding(SupportsShouldProcess)]
param(
    [Parameter(ParameterSetName='Set', Mandatory)]
    [ValidatePattern('^\d+\.\d+\.\d+$')]
    [string]$Version,

    [Parameter(ParameterSetName='Set')]
    [int]$VersionCode,

    # -Check：只读校验四派生点一致，不写入。供 build-installer / CI 末尾 gate。
    [Parameter(ParameterSetName='Check', Mandatory)]
    [switch]$Check
)

$ErrorActionPreference = 'Stop'

# 版本单一事实源是仓库根 version.json。派生点分两类：
#   1) 结构化派生点（JSON 字段）——由本脚本写入；
#   2) 文本声明点（README/文档里的「当前版本」字样）——由本脚本校验，人工修订。
# Android 侧不在任一类中：app/build.gradle.kts 构建期直接读 version.json（拉而非推）。
$sourceRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$versionFile = Join-Path $sourceRoot 'version.json'
$foxTauriFile = Join-Path $sourceRoot 'platforms\windows-installer\tauri.conf.json'
$tauriFile = Join-Path $sourceRoot 'platforms\windows-settings\tauri.conf.json'
$npmFile = Join-Path $sourceRoot 'platforms\windows-settings\package.json'
$gradleProps = Join-Path $sourceRoot 'platforms\android\gradle.properties'

# 文本声明点表。新增一处「写死版本号的文档」时在此加一行即可，无需改校验逻辑。
# Pattern 必须含一个捕获组，捕到的就是该处声明的版本号。
$textClaims = @(
    @{ Path = 'README.md'; Pattern = '状态-v(\d+\.\d+\.\d+)-'; Desc = 'README 状态徽章' }
    @{ Path = 'README.md'; Pattern = '当前版本 \*\*v(\d+\.\d+\.\d+)'; Desc = 'README 当前状态段' }
)

function Read-VersionManifest {
    $manifest = Get-Content -LiteralPath $versionFile -Raw | ConvertFrom-Json
    return [pscustomobject]@{
        Version = [string]$manifest.version
        VersionCode = [int]$manifest.versionCode
    }
}

function Assert-VersionConsistency {
    $manifest = Read-VersionManifest

    $foxTauri = Get-Content -LiteralPath $foxTauriFile -Raw | ConvertFrom-Json
    $tauri = Get-Content -LiteralPath $tauriFile -Raw | ConvertFrom-Json
    $npm = Get-Content -LiteralPath $npmFile -Raw | ConvertFrom-Json

    $mismatch = @()
    if ([string]$foxTauri.version -ne $manifest.Version) { $mismatch += "FOX安装器 tauri.conf.json=$($foxTauri.version)" }
    if ([string]$tauri.version -ne $manifest.Version) { $mismatch += "设置页 tauri.conf.json=$($tauri.version)" }
    if ([string]$npm.version -ne $manifest.Version) { $mismatch += "package.json=$($npm.version)" }
    # 安装器页面标题栏的品牌版本号（如 "FOX输入法 0.4.1"）至少一处与 version.json 对齐
    $uiDir = Join-Path $sourceRoot 'platforms\windows-installer\ui'
    $uiVersionOk = Get-ChildItem -LiteralPath $uiDir -Filter '*.html' -ErrorAction SilentlyContinue |
        Where-Object { (Get-Content -LiteralPath $_.FullName -Raw) -match "FOX输入法 $([regex]::Escape($manifest.Version))" } |
        Select-Object -First 1
    if (-not $uiVersionOk) { $mismatch += "安装器页面版本串" }

    # 文本声明点：README 等文档里写死的「当前版本」。这类漂移曾同时存在三个
    # 不同版本号（version.json=1.8.0 / 徽章=0.8.0 / 正文=1.7.0）而无人察觉。
    foreach ($claim in $textClaims) {
        $full = Join-Path $sourceRoot $claim.Path
        if (-not (Test-Path -LiteralPath $full)) {
            $mismatch += "$($claim.Desc)：文件缺失 $($claim.Path)"
            continue
        }
        $text = Get-Content -LiteralPath $full -Raw
        $m = [regex]::Match($text, $claim.Pattern)
        if (-not $m.Success) {
            $mismatch += "$($claim.Desc)：未匹配到版本号（模式 $($claim.Pattern)）"
        }
        elseif ($m.Groups[1].Value -ne $manifest.Version) {
            $mismatch += "$($claim.Desc)=$($m.Groups[1].Value)"
        }
    }

    # CHANGELOG 必须已为当前版本开条目，避免「发了版但没记账」。
    $changelog = Join-Path $sourceRoot 'CHANGELOG.md'
    if (Test-Path -LiteralPath $changelog) {
        $clText = Get-Content -LiteralPath $changelog -Raw
        if ($clText -notmatch "(?m)^## \[$([regex]::Escape($manifest.Version))\]") {
            $mismatch += "CHANGELOG.md 缺少 ## [$($manifest.Version)] 条目"
        }
    }

    # gradle.properties 曾因固定 SHURUFA_VERSION_* 产生过 0.4.0/9 vs 0.4.1/10 漂移；
    # 此后该属性应永久移除，此处做主动护栏。本地试包请用 -PSHURUFA_VERSION_CODE=n 命令行覆盖。
    # 必须行首锚定：否则本文件注释里提到该属性名时会误触发。
    if ($gradleProps -and (Test-Path -LiteralPath $gradleProps)) {
        $gp = Get-Content -LiteralPath $gradleProps -Raw
        if ($gp -match '(?m)^\s*SHURUFA_VERSION_(CODE|NAME)=') {
            throw "gradle.properties 仍固定 SHURUFA_VERSION_*，会导致与 version.json 漂移。请删除这些行。"
        }
    }

    if ($mismatch.Count -gt 0) {
        throw "版本不一致：version.json=$($manifest.Version)，但 $($mismatch -join '，')。结构化派生点请运行 set-version.ps1 对齐，文本声明点需人工修订。"
    }
    return $manifest
}


function Write-Utf8NoBom([string]$Path, [string]$Text) {
    # PS 5.1 的 Set-Content -Encoding utf8 会写 BOM，破坏 serde_json 等解析器
    [System.IO.File]::WriteAllText($Path, $Text, (New-Object System.Text.UTF8Encoding($false)))
}

$current = Assert-VersionConsistency

if ($Check) {
    Write-Host "版本一致：$($current.Version)/$($current.VersionCode)"
    exit 0
}

if (-not $PSBoundParameters.ContainsKey('VersionCode')) {
    # 未显式指定时按 Android 规则递增 versionCode；同版本重打包必须显式传当前值。
    if ($Version -ne $current.Version) {
        $VersionCode = $current.VersionCode + 1
    }
    else {
        throw "版本号未变化（仍为 $($current.Version)）。重打包同版本时请显式指定 -VersionCode $($current.VersionCode)。"
    }
}

if ($PSCmdlet.ShouldProcess("Shurufa $($current.Version)/$($current.VersionCode) -> $Version/$VersionCode", '更新版本')) {
    Write-Utf8NoBom $versionFile (([pscustomobject]@{ version = $Version; versionCode = $VersionCode } | ConvertTo-Json))

    $foxTauri = Get-Content -LiteralPath $foxTauriFile -Raw | ConvertFrom-Json
    $foxTauri.version = $Version
    Write-Utf8NoBom $foxTauriFile ($foxTauri | ConvertTo-Json -Depth 32)

    # FOX 安装器页面里的品牌版本号（标题栏/完成标题），如 "FOX输入法 0.4.1"。
    Get-ChildItem -LiteralPath (Join-Path $sourceRoot 'platforms\windows-installer\ui') -Filter '*.html' |
        ForEach-Object {
            $html = Get-Content -LiteralPath $_.FullName -Raw
            $html = $html -replace 'FOX输入法 \d+\.\d+\.\d+', "FOX输入法 $Version"
            Write-Utf8NoBom $_.FullName $html
        }

    $tauri = Get-Content -LiteralPath $tauriFile -Raw | ConvertFrom-Json
    $tauri.version = $Version
    Write-Utf8NoBom $tauriFile ($tauri | ConvertTo-Json -Depth 32)

    $npm = Get-Content -LiteralPath $npmFile -Raw | ConvertFrom-Json
    $npm.version = $Version
    Write-Utf8NoBom $npmFile ($npm | ConvertTo-Json -Depth 32)

    # 不再向 gradle.properties 写入版本：Android 侧由 build.gradle.kts 构建期
    # 直接读 version.json（拉而非推），此处固定值只会造成漂移（曾 0.4.0/9 vs 0.4.1/10）。
    # 若该文件残留这两行，由 Assert-VersionConsistency 触发删除提示。
    # 必须走 Write-Utf8NoBom：-Encoding ascii 会把本文件的中文注释毁成问号。
    $gradleText = Get-Content -LiteralPath $gradleProps -Raw
    $gradleText = $gradleText -replace '(?m)^SHURUFA_VERSION_CODE=\d+\r?\n', ''
    $gradleText = $gradleText -replace '(?m)^SHURUFA_VERSION_NAME=[^\r\n]*\r?\n', ''
    $gradleText = $gradleText.TrimEnd("`r", "`n") + "`r`n"
    Write-Utf8NoBom $gradleProps $gradleText
}

Assert-VersionConsistency | Out-Null
Write-Host "版本已对齐：version=$Version, versionCode=$VersionCode"
