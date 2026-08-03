[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$StatePath,
    [switch]$Restore
)

$ErrorActionPreference = 'Stop'
$shurufaInputTip = '0804:{8A5C1B49-3D2E-4F7A-9C61-0B7E2D5A9F13}{C4E9D2A7-6B31-4A58-8F0D-1E9A7C3B5D26}'
$emptyOverrideMarker = '__SHURUFA_NO_INPUT_METHOD_OVERRIDE__'

if ($Restore) {
    if (-not (Test-Path -LiteralPath $StatePath -PathType Leaf)) {
        exit 0
    }
    $previousInputTip = (Get-Content -LiteralPath $StatePath -Raw).Trim()
    if ($previousInputTip -eq $emptyOverrideMarker) {
        Set-WinDefaultInputMethodOverride
    }
    elseif ($previousInputTip) {
        Set-WinDefaultInputMethodOverride -InputTip $previousInputTip
    }
    exit 0
}

$previousInputTip = (Get-WinDefaultInputMethodOverride).InputMethodTip
if ([string]::IsNullOrWhiteSpace($previousInputTip)) {
    $previousInputTip = $emptyOverrideMarker
}

$stateDirectory = Split-Path -Parent $StatePath
if ($stateDirectory) {
    New-Item -ItemType Directory -Path $stateDirectory -Force | Out-Null
}
Set-Content -LiteralPath $StatePath -Value $previousInputTip -NoNewline -Encoding utf8
Set-WinDefaultInputMethodOverride -InputTip $shurufaInputTip

if ((Get-WinDefaultInputMethodOverride).InputMethodTip -ne $shurufaInputTip) {
    throw 'Windows 未确认 Shurufa 拼音已成为默认输入法。'
}
