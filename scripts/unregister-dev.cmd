@echo off
rem 开发期反注册：从系统移除 TSF 文本服务（需管理员权限运行）
setlocal
set DLL=%~dp0..\target\debug\shurufa_tsf.dll
regsvr32 /u "%DLL%"
endlocal
