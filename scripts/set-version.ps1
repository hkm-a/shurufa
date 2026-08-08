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
# NSIS、Tauri、Android 兜底属性三处派生点，并在写入前后做一致性校验。
$sourceRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$versionFile = Join-Path $sourceRoot 'version.json'
$nsiFile = Join-Path $sourceRoot 'installer\shurufa.nsi'
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

    $nsi = Get-Content -LiteralPath $nsiFile -Raw
    if ($nsi -notmatch '!define\s+PRODUCT_VERSION\s+"([^"]+)"') {
        throw "无法在 $nsiFile 中找到 PRODUCT_VERSION 定义。"
    }
    $nsiVersion = $Matches[1]

    $tauri = Get-Content -LiteralPath $tauriFile -Raw | ConvertFrom-Json
    $npm = Get-Content -LiteralPath $npmFile -Raw | ConvertFrom-Json

    $mismatch = @()
    if ($nsiVersion -ne $manifest.Version) { $mismatch += "NSIS=$nsiVersion" }
    if ([string]$tauri.version -ne $manifest.Version) { $mismatch += "tauri.conf.json=$($tauri.version)" }
    if ([string]$npm.version -ne $manifest.Version) { $mismatch += "package.json=$($npm.version)" }

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
    [pscustomobject]@{ version = $Version; versionCode = $VersionCode } |
        ConvertTo-Json | Set-Content -LiteralPath $versionFile -Encoding utf8

    $nsiText = Get-Content -LiteralPath $nsiFile -Raw
    $nsiText = $nsiText -replace '!define\s+PRODUCT_VERSION\s+"[^"]+"', "!define PRODUCT_VERSION `"$Version`""
    Set-Content -LiteralPath $nsiFile -Value $nsiText -Encoding utf8 -NoNewline

    $tauri = Get-Content -LiteralPath $tauriFile -Raw | ConvertFrom-Json
    $tauri.version = $Version
    $tauri | ConvertTo-Json -Depth 32 | Set-Content -LiteralPath $tauriFile -Encoding utf8

    $npm = Get-Content -LiteralPath $npmFile -Raw | ConvertFrom-Json
    $npm.version = $Version
    $npm | ConvertTo-Json -Depth 32 | Set-Content -LiteralPath $npmFile -Encoding utf8

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
