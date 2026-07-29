@echo off
rem Start the clipboard listener daemon (minimized console).
setlocal
set EXE=%~dp0..\target\debug\shurufa-host.exe
if not exist "%EXE%" (
    echo [error] build first: cargo build -p shurufa-host
    pause
    exit /b 1
)
start "shurufa-host" /min "%EXE%" run
echo Listener started. Panel hotkey: Ctrl+Shift+V (or Alt+V if occupied).
endlocal
