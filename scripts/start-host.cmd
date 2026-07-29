@echo off
rem Start the clipboard listener daemon (minimized console).
rem Kills any stale instance first: a stale one would keep the hotkey
rem registered and force the new instance onto the fallback key.
setlocal
set EXE=%~dp0..\target\debug\shurufa-host.exe
if not exist "%EXE%" (
    echo [error] build first: cargo build -p shurufa-host
    pause
    exit /b 1
)
taskkill /im shurufa-host.exe /f >nul 2>&1
start "shurufa-host" /min "%EXE%" run
echo Listener started. Panel hotkey: Ctrl+Shift+V (or Alt+V if occupied).
echo Log file: %%TEMP%%\shurufa-host.log
endlocal
