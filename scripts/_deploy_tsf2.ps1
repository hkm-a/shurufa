$ErrorActionPreference = 'Continue'
$src = 'C:\Users\hkm\Documents\shurufa\target\debug\shurufa_tsf.dll'
$dir = Join-Path $env:ProgramData 'shurufa'
$dest = Join-Path $dir 'shurufa_tsf.dll'
taskkill /f /im TextInputHost.exe 2>$null | Out-Null
taskkill /f /im ctfmon.exe 2>$null | Out-Null
Start-Sleep -Milliseconds 800
if (Test-Path $dest) { Rename-Item -LiteralPath $dest -NewName ("shurufa_tsf.dll." + [DateTime]::Now.Ticks + ".stale") -Force }
Copy-Item -LiteralPath $src -Destination $dest -Force
icacls "$dir" /grant *S-1-15-2-1:(OI)(CI)(RX) /t /c | Out-Null
regsvr32 /s $dest
Start-Process ctfmon.exe
"done"
