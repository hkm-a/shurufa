@echo off
rem Unregister the TSF IME and remove the deployed binaries. Run as Administrator.
rem User dictionary / sync data under schemas\user and rime-user-data is preserved
rem unless the environment variable SHURUFA_PURGE_DATA=1 is set before running.
setlocal
set DEST=%ProgramData%\shurufa

if exist "%DEST%\shurufa-host.exe" (
    "%DEST%\shurufa-host.exe" stop >nul 2>nul
)
reg delete "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" /v ShurufaHost /f >nul 2>nul
if exist "%DEST%\shurufa_tsf.dll" (
    regsvr32 /u "%DEST%\shurufa_tsf.dll"
) else (
    regsvr32 /u "%~dp0..\target\debug\shurufa_tsf.dll"
)
if /i "%SHURUFA_PURGE_DATA%"=="1" (
    rem The DLL may stay mapped in running apps; deletion can fail until sign-out.
    rd /s /q "%DEST%" 2>nul
) else (
    rem Keep user data: remove binaries but leave schemas\user & logs intact.
    del /q "%DEST%\*.exe" "%DEST%\*.dll" 2>nul
    echo User data under %DEST% preserved. Set SHURUFA_PURGE_DATA=1 to remove everything.
)
endlocal
