@echo off
setlocal
set DLL=%~dp0..\target\debug\shurufa_tsf.dll
if not exist "%DLL%" (
    echo [error] no %DLL%£¬please execute this first: cargo build -p shurufa-tsf
    pause
    exit /b 1
)
regsvr32 "%DLL%"
pause
endlocal