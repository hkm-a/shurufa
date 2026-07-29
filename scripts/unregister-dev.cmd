@echo off
rem Unregister the TSF IME and remove the deployed copy. Run as Administrator.
setlocal
set DEST=%ProgramData%\shurufa

if exist "%DEST%\shurufa_tsf.dll" (
    regsvr32 /u "%DEST%\shurufa_tsf.dll"
) else (
    regsvr32 /u "%~dp0..\target\debug\shurufa_tsf.dll"
)
rem The DLL may stay mapped in running apps; deletion can fail until sign-out.
rd /s /q "%DEST%" 2>nul
endlocal
