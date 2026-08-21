$ErrorActionPreference = 'Continue'
$log = 'C:\Users\hkm\Documents\shurufa\dist\ai-deploy.log'
"start $(Get-Date -Format o)" | Out-File $log -Encoding utf8
$src = 'C:\Users\hkm\Documents\shurufa\target\debug\shurufa_tsf.dll'
$dir = Join-Path $env:ProgramData 'shurufa'
$dest = Join-Path $dir 'shurufa_tsf.dll'
if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir | Out-Null }
taskkill /f /im TextInputHost.exe 2>$null | Out-Null
taskkill /f /im ctfmon.exe 2>$null | Out-Null
Start-Sleep -Milliseconds 800
if (Test-Path $dest) {
    $stale = "$dest.$([DateTime]::Now.Ticks).stale"
    Rename-Item -LiteralPath $dest -NewName (Split-Path $stale -Leaf) -Force
    "renamed old" | Out-File $log -Append
}
Copy-Item -LiteralPath $src -Destination $dest -Force
"copied" | Out-File $log -Append
icacls "$dir" /grant *S-1-15-2-1:(OI)(CI)(RX) /t /c | Out-Null
"icacls ok" | Out-File $log -Append
regsvr32 /s $dest
"regsvr32 exit=$LASTEXITCODE" | Out-File $log -Append
Start-Process ctfmon.exe
"done" | Out-File $log -Append
