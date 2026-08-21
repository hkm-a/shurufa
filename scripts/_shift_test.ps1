$ErrorActionPreference = "Continue"
$env:AGNES_API_KEY = [Environment]::GetEnvironmentVariable("AGNES_API_KEY","User")
Set-Clipboard -Value ""
Get-Process chrome -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 1
$html = "<html><body style=background:#f0f0f0><textarea id=t rows=8 cols=80 autofocus style=font-size:20px;margin:40px></textarea></body></html>"
$hp = Join-Path $env:TEMP "shiftest.html"
Set-Content -Path $hp -Value $html -Encoding UTF8
Start-Process "C:\Program Files\Google\Chrome\Application\chrome.exe" -ArgumentList $hp, "--disable-session-crashed-bubble", "--no-restore-session-state"
Start-Sleep -Seconds 4
$sh = New-Object -ComObject WScript.Shell
$ok = $false
for ($i = 0; $i -lt 10; $i++) { $ok = $sh.AppActivate("shiftest.html"); if ($ok) { break }; Start-Sleep -Milliseconds 600 }
"activated: " + $ok
Start-Sleep -Milliseconds 800
$sh.SendKeys("nihao")
Start-Sleep -Milliseconds 800
$sh.SendKeys("{SHIFT}")
Start-Sleep -Milliseconds 300
$sh.SendKeys("{SHIFT up}")
Start-Sleep -Milliseconds 800
$sh.SendKeys("^a")
Start-Sleep -Milliseconds 200
$sh.SendKeys("^c")
Start-Sleep -Milliseconds 300
$clip = Get-Clipboard -Raw
"textarea: [" + $clip + "]"