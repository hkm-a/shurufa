@echo off
chcp 65001 >nul
echo === TIP Registry ===
reg query "HKLM\SOFTWARE\Microsoft\CTF\TIP\{8A5C1B49-3D2E-4F7A-9C61-0B7E2D5A9F13}" /s
echo.
echo === Category Registry ===
reg query "HKLM\SOFTWARE\Microsoft\CTF\TIP\{8A5C1B49-3D2E-4F7A-9C61-0B7E2D5A9F13}\Category" /s
echo.
echo === LanguageProfile Registry ===
reg query "HKLM\SOFTWARE\Microsoft\CTF\TIP\{8A5C1B49-3D2E-4F7A-9C61-0B7E2D5A9F13}\LanguageProfile" /s
pause
