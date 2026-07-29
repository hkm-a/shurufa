@echo off
rem Deploy and register the TSF IME. Run as Administrator.
rem Copies the DLLs and schema data to %ProgramData%\shurufa so that
rem AppContainer (immersive) processes such as TextInputHost.exe can
rem load them; DLLs under the user profile are NOT readable there.
rem Re-run this script after every rebuild to refresh the deployed copy.
setlocal
set SRC=%~dp0..
set DEST=%ProgramData%\shurufa

if not exist "%SRC%\target\debug\shurufa_tsf.dll" (
    echo [error] build first: cargo build -p shurufa-tsf
    pause
    exit /b 1
)
if not exist "%SRC%\third_party\librime\dist\lib\rime.dll" (
    echo [error] missing third_party\librime, see README for the download command
    pause
    exit /b 1
)

if not exist "%DEST%" mkdir "%DEST%"
copy /y "%SRC%\target\debug\shurufa_tsf.dll" "%DEST%\" >nul || goto :copyfail
copy /y "%SRC%\third_party\librime\dist\lib\rime.dll" "%DEST%\" >nul || goto :copyfail
xcopy /e /i /y "%SRC%\schemas" "%DEST%\schemas" >nul || goto :copyfail

rem Grant read+execute to ALL APPLICATION PACKAGES (SID S-1-15-2-1)
icacls "%DEST%" /grant *S-1-15-2-1:(OI)(CI)(RX) /t >nul

regsvr32 "%DEST%\shurufa_tsf.dll"
pause
endlocal
exit /b 0

:copyfail
echo [error] copy failed. If the IME is in use, sign out and back in, then retry.
pause
exit /b 1
