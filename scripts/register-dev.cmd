@echo off
rem 开发期注册：把 debug 构建的 TSF 文本服务注册进系统（需管理员权限运行）
setlocal
set DLL=%~dp0..\target\debug\shurufa_tsf.dll
if not exist "%DLL%" (
    echo [错误] 未找到 %DLL%，请先执行 cargo build -p shurufa-tsf
    exit /b 1
)
regsvr32 "%DLL%"
endlocal
