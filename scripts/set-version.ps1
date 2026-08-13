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

# 版本单一事实源是仓库根 version.json；本脚本负责把新值同步到
# FOX 安装器、设置页 Tauri、npm 三处派生点，并在写入前后做一致性校验。
$sourceRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$versionFile = Join-Path $sourceRoot 'version.json'
$foxTauriFile = Join-Path $sourceRoot 'platforms\windows-installer\tauri.conf.json'
$tauriFile = Join-Path $sourceRoot 'platforms\windows-settings\tauri.conf.json'
$npmFile = Join-Path $sourceRoot 'platforms\windows-settings\package.json'
$gradleProps = Join-Path $sourceRoot 'platforms\android\gradle.properties'

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

    # gradle.properties 曾因固定 SHURUFA_VERSION_* 产生过 0.4.0/9 vs 0.4.1/10 漂移；
    # 此后该属性应永久移除，此处做主动护栏。若真有本地覆盖需求，请用 -PversionCodeOverride 属性而非此处。
    if ($gradleProps -and (Test-Path -LiteralPath $gradleProps)) {
        $gp = Get-Content -LiteralPath $gradleProps -Raw
        if ($gp -match 'SHURUFA_VERSION_(CODE|NAME)=') {
            throw "gradle.properties 仍固定 SHURUFA_VERSION_*，会导致与 version.json 漂移。请删除这些行。"
        }
    }

    if ($mismatch.Count -gt 0) {
        throw "版本不一致：version.json=$($manifest.Version)，但 $($mismatch -join '，')。请先运行 set-version.ps1 对齐。"
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

    # 不再向 gradle.properties 写入版本：它会与 version.json 漂移（曾 0.4.0/9 vs 0.4.1/10）。
    # 若该文件残留这两行，由 Assert-VersionConsistency 触发删除提示。
    $gradleText = Get-Content -LiteralPath $gradleProps -Raw
    $gradleText = $gradleText -replace '(?m)^SHURUFA_VERSION_CODE=\d+\r?\n', ''
    $gradleText = $gradleText -replace '(?m)^SHURUFA_VERSION_NAME=[^\r\n]*\r?\n', ''
    $gradleText = $gradleText.TrimEnd("`r", "`n") + "`r`n"
    Set-Content -LiteralPath $gradleProps -Value $gradleText -Encoding ascii -NoNewline
}

Assert-VersionConsistency | Out-Null
Write-Host "版本已对齐：version=$Version, versionCode=$VersionCode"
