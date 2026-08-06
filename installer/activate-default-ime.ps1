[CmdletBinding()]
param(
    [switch]$Clear
)

$ErrorActionPreference = 'Stop'
$shurufaInputTip = '0804:{8A5C1B49-3D2E-4F7A-9C61-0B7E2D5A9F13}{C4E9D2A7-6B31-4A58-8F0D-1E9A7C3B5D26}'

if ($Clear) {
    # 卸载时仅在覆盖仍指向 Shurufa 时清除，避免动到用户后来改选的其他输入法。
    if ((Get-WinDefaultInputMethodOverride).InputMethodTip -eq $shurufaInputTip) {
        Set-WinDefaultInputMethodOverride
    }
    exit 0
}

Set-WinDefaultInputMethodOverride -InputTip $shurufaInputTip
if ((Get-WinDefaultInputMethodOverride).InputMethodTip -ne $shurufaInputTip) {
    throw 'Windows 未确认 Shurufa 拼音已成为默认输入法。'
}
