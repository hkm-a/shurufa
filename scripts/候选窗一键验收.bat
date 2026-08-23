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

if not exist "target\debug\shurufa-ui.exe" (
    echo [1/3] 第一次运行，先构建 shurufa-ui...
    cargo build -p shurufa-host --bin shurufa-ui
    if errorlevel 1 (
        echo.
        echo [错误] 构建失败，请确认已安装 Rust 工具链。
        pause
        exit /b 1
    )
)

rem 自动安装 pywinauto（验收脚本依赖）
%PY% -c "import pywinauto" >nul 2>&1
if errorlevel 1 (
    echo [2/3] 安装 pywinauto...
    %PY% -m pip install pywinauto
    if errorlevel 1 (
        echo [错误] pywinauto 安装失败，请检查网络。
        pause
        exit /b 1
    )
)

echo [3/3] 开始自动验收（会自动弹候选窗，请勿动鼠标键盘）...
echo.
%PY% scripts\test-cand-acceptance.py --exe target\debug\shurufa-ui.exe
set EXIT_CODE=%ERRORLEVEL%
echo.
if "%EXIT_CODE%"=="0" (
    echo ============================================
    echo   结果：全部通过
    echo ============================================
) else (
    echo ============================================
    echo   结果：存在失败，请看上面 FAIL 信息
    echo ============================================
)
echo.
pause
