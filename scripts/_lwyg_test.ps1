$ErrorActionPreference = "Continue"
$env:AGNES_API_KEY = [Environment]::GetEnvironmentVariable("AGNES_API_KEY","User")
Get-Process chrome -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 1
$html = "<html><body style=background:#f0f0f0><textarea id=t rows=8 cols=80 autofocus style=font-size:20px;margin:40px></textarea></body></html>"
$hp = Join-Path $env:TEMP "lwygtest.html"
Set-Content -Path $hp -Value $html -Encoding UTF8
Start-Process "C:\Program Files\Google\Chrome\Application\chrome.exe" -ArgumentList $hp, "--disable-session-crashed-bubble", "--noerrdialogs"
Start-Sleep -Seconds 5
$sh = New-Object -ComObject WScript.Shell
$ok = $false
for ($i = 0; $i -lt 10; $i++) { $ok = $sh.AppActivate("lwygtest.html"); if ($ok) { break }; Start-Sleep -Milliseconds 600 }
"activated: " + $ok
Start-Sleep -Milliseconds 800
$sh.SendKeys("lwyg")
"typed lwyg"
Start-Sleep -Seconds 3
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$vs = [System.Windows.Forms.SystemInformation]::VirtualScreen
$bmp = New-Object System.Drawing.Bitmap($vs.Width, $vs.Height)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($vs.X, $vs.Y, 0, 0, $bmp.Size)
$bmp.Save("C:\Users\hkm\Documents\shurufa\dist\win-lwyg.png", [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
"shot saved"