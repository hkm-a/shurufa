@echo off
chcp 65001 >nul
cd /d "%~dp0.."
echo ============================================
echo   候选窗一键验收（小白模式）
echo ============================================
echo.

rem 自动选择 Python 命令
set "PY=python"
python --version >nul 2>&1
if errorlevel 1 (
    set "PY=py -3"
    py -3 --version >nul 2>&1
    if errorlevel 1 (
        echo [错误] 未找到 Python，请先安装 Python 3。
        pause
        exit /b 1
    )
)

echo [1/5] 构建 shurufa-ui 与 shurufa-tsf...
cargo build -p shurufa-host --bin shurufa-ui
if errorlevel 1 (
    echo.
    echo [错误] shurufa-ui 构建失败，请确认已安装 Rust 工具链。
    pause
    exit /b 1
)
cargo build -p shurufa-tsf
if errorlevel 1 (
    echo.
    echo [错误] shurufa-tsf 构建失败，请确认已安装 Rust 工具链。
    pause
    exit /b 1
)

rem 检查已部署的 TSF 是否比刚构建的旧（真实 hosted 需要 TSF 支持 candidate_window）
if exist "%ProgramData%\shurufa\shurufa_tsf.dll" (
    for %%A in ("%ProgramData%\shurufa\shurufa_tsf.dll") do set DEPLOYED_TSF=%%~tA
    for %%A in ("target\debug\shurufa_tsf.dll") do set BUILT_TSF=%%~tA
    echo [提醒] 已安装 TSF: %DEPLOYED_TSF%
    echo [提醒] 本次构建 TSF: %BUILT_TSF%
    echo [提醒] 若已安装版本较旧，真实 hosted 验收可能失败。
    echo        请以管理员身份运行 scripts\register-dev.cmd 部署最新 TSF。
    echo.
)

rem 自动安装 pywinauto（验收脚本依赖）
%PY% -c "import pywinauto" >nul 2>&1
if errorlevel 1 (
    echo [2/5] 安装 pywinauto...
    %PY% -m pip install pywinauto
    if errorlevel 1 (
        echo [错误] pywinauto 安装失败，请检查网络。
        pause
        exit /b 1
    )
)

echo [3/5] 开始自动验收（会自动弹候选窗，请勿动鼠标键盘）...
echo.
%PY% scripts\test-cand-acceptance.py --exe target\debug\shurufa-ui.exe
set EXIT_CODE=%ERRORLEVEL%
echo.

if not "%EXIT_CODE%"=="0" (
    echo 自动验收存在失败，跳过手动项自动验收。
    echo 请看上面 FAIL 信息。
    pause
    exit /b 1
)

echo [4/5] 开始单屏手动项自动验收（脚本自动打开 Notepad4 并确认，无需人工按键）...
echo.
%PY% scripts\test-cand-manual-auto.py --exe target\debug\shurufa-ui.exe
set MANUAL_CODE=%ERRORLEVEL%
echo.
if "%MANUAL_CODE%"=="0" (
    echo ============================================
    echo   结果：自动 + 手动项全部通过
    echo ============================================
) else (
    echo ============================================
    echo   结果：手动项自动验收存在失败，请看上面 FAIL 信息
    echo   常见原因：已部署 TSF 过旧，请先以管理员运行 scripts\register-dev.cmd
    echo ============================================
)
echo.
pause
