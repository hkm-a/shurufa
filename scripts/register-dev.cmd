@echo off
rem Deploy and register the TSF IME. Run as Administrator.
rem Copies the DLLs and schema data to %ProgramData%\shurufa so that
rem AppContainer (immersive) processes such as TextInputHost.exe can
rem load them; DLLs under the user profile are NOT readable there.
rem Re-run this script after every rebuild to refresh the deployed copy.
setlocal enabledelayedexpansion
set SRC=%~dp0..
set DEST=%ProgramData%\shurufa

if not exist "%SRC%\target\debug\shurufa_tsf.dll" (
    echo [error] build first: cargo build -p shurufa-tsf
    pause
    exit /b 1
)
if not exist "%SRC%\target\debug\shurufa-algo.exe" (
    echo [error] build first: cargo build -p shurufa-algo
    pause
    exit /b 1
)
if not exist "%SRC%\third_party\librime\dist\lib\rime.dll" (
    echo [error] missing third_party\librime, see README for the download command
    pause
    exit /b 1
)

if not exist "%DEST%" mkdir "%DEST%"

rem 阶段 3：生成三份构建期产物（已出库），再复制 schemas。
powershell -NoProfile -ExecutionPolicy Bypass -File "%SRC%\scripts\regenerate-generated.ps1" >nul || goto :copyfail

rem Loaded DLLs and memory-mapped dictionaries cannot be overwritten,
rem but they CAN be renamed. Move live files out of the way first and
rem sweep leftovers from previous runs (best effort; locked ones stay).
del /q "%DEST%\*.stale" 2>nul
del /q "%DEST%\schemas\build\*.stale" 2>nul
if exist "%DEST%\shurufa_tsf.dll" ren "%DEST%\shurufa_tsf.dll" "shurufa_tsf.dll.!RANDOM!.stale" 2>nul
if exist "%DEST%\rime.dll" ren "%DEST%\rime.dll" "rime.dll.!RANDOM!.stale" 2>nul
if exist "%DEST%\schemas\build" (
    for %%f in ("%DEST%\schemas\build\*") do ren "%%f" "%%~nxf.!RANDOM!.stale" 2>nul
)

copy /y "%SRC%\target\debug\shurufa_tsf.dll" "%DEST%\" >nul || goto :copyfail
copy /y "%SRC%\third_party\librime\dist\lib\rime.dll" "%DEST%\" >nul || goto :copyfail
copy /y "%SRC%\third_party\librime\dist\bin\rime_deployer.exe" "%DEST%\" >nul || goto :copyfail
xcopy /e /i /y "%SRC%\schemas" "%DEST%\schemas" >nul || goto :copyfail

rem Precompile dictionaries so the in-host engine never spends tens of
rem seconds compiling on the first keystroke (that freeze once got the
rem IME kicked out of the keyboard list by the input service).
echo Precompiling dictionaries, this may take a while...
pushd "%DEST%"
rime_deployer.exe --build "%DEST%\schemas" "%DEST%\schemas" "%DEST%\schemas\build" >nul 2>&1
popd
if not exist "%DEST%\schemas\build\luna_pinyin.table.bin" (
    echo [error] dictionary precompile failed
    pause
    exit /b 1
)

rem Grant read+execute to ALL APPLICATION PACKAGES (SID S-1-15-2-1)
icacls "%DEST%" /grant *S-1-15-2-1:(OI)(CI)(RX) /t >nul

regsvr32 "%DEST%\shurufa_tsf.dll"
echo.
echo Done. If the keyboard is missing from the list, re-add it in:
echo   Settings - Language - Chinese (Simplified) - Keyboards - Add "Shurufa Pinyin"
pause
endlocal
exit /b 0

:copyfail
echo [error] copy failed. If the IME is in use, sign out and back in, then retry.
pause
exit /b 1
