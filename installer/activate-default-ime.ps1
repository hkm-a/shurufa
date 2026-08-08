[CmdletBinding()]
param(
    [switch]$Clear
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Deploy-Shurufa.ps1')

if ($Clear) {
    Clear-ShurufaDefaultIme
    exit 0
}

Set-ShurufaDefaultIme
