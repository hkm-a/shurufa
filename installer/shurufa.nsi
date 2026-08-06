Unicode true
RequestExecutionLevel admin

!include "MUI2.nsh"

!define PRODUCT_NAME "Shurufa 拼音"
!define PRODUCT_VERSION "0.4.1"
!define PRODUCT_PUBLISHER "Shurufa"
!define PRODUCT_REGISTRY_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Shurufa"
!define TSF_DLL_FILE "shurufa_tsf-${PRODUCT_VERSION}.dll"

Name "${PRODUCT_NAME} ${PRODUCT_VERSION}"
OutFile "..\dist\Shurufa-Setup.exe"
InstallDir "$APPDATA\shurufa"
ShowInstDetails nevershow
ShowUninstDetails nevershow

!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_NOAUTOCLOSE
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"

Var RegistrationTool
Var InputMethodTool
Var CtfmonTool
Var InstallLog

; 尽力而为步骤（目标进程/注册项可能本就不存在），结果不判定成败。
!macro RunHidden Command
  nsExec::ExecToLog `${Command}`
  Pop $0
!macroend

; 安装日志一行（UTF-16LE 保证中文可读）。日志句柄无效时静默丢弃，不影响安装。
!macro LogLine Text
  FileWriteUTF16LE $InstallLog `${Text}$\r$\n`
!macroend

; 关键步骤：nsExec 拉不起工具时返回的是字符串 error/timeout，IntCmp 会把它们按
; 数值 0 误判为成功（曾导致自启动注册静默丢失），必须与字符串 "0" 严格比较；
; 失败弹窗附带真实结果值并中止安装，静默安装（/S）经 /SD 取默认按钮。
!macro RunCritical Command Message
  nsExec::ExecToLog `${Command}`
  Pop $0
  !insertmacro LogLine `结果=$0 步骤=${Message}`
  StrCmp $0 "0" +3
  MessageBox MB_ICONSTOP `${Message}失败（结果：$0）。现有安装目录不会被删除，请关闭占用程序后重试。` /SD IDOK
  Abort
!macroend

Function .onInit
  SetShellVarContext all
  StrCpy $INSTDIR "$APPDATA\shurufa"
  ; 32 位安装进程访问 System32 会被 WOW64 重定向到 SysWOW64，必须经 Sysnative
  ; 使用 64 位工具：regsvr32 注册 64 位 TSF DLL、ctfmon 与 PowerShell 同理。
  IfFileExists "$WINDIR\Sysnative\regsvr32.exe" 0 use_default_tools
  StrCpy $RegistrationTool "$WINDIR\Sysnative\regsvr32.exe"
  StrCpy $InputMethodTool "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
  StrCpy $CtfmonTool "$WINDIR\Sysnative\ctfmon.exe"
  Goto tools_ready
use_default_tools:
  StrCpy $RegistrationTool "$SYSDIR\regsvr32.exe"
  StrCpy $InputMethodTool "$SYSDIR\WindowsPowerShell\v1.0\powershell.exe"
  StrCpy $CtfmonTool "$SYSDIR\ctfmon.exe"
tools_ready:
FunctionEnd

Function un.onInit
  IfFileExists "$WINDIR\Sysnative\regsvr32.exe" 0 un_use_default_tools
  StrCpy $RegistrationTool "$WINDIR\Sysnative\regsvr32.exe"
  StrCpy $InputMethodTool "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
  Goto un_tools_ready
un_use_default_tools:
  StrCpy $RegistrationTool "$SYSDIR\regsvr32.exe"
  StrCpy $InputMethodTool "$SYSDIR\WindowsPowerShell\v1.0\powershell.exe"
un_tools_ready:
FunctionEnd

Function StopInputProcesses
  !insertmacro RunHidden `"$SYSDIR\taskkill.exe" /f /im ctfmon.exe`
  !insertmacro RunHidden `"$SYSDIR\taskkill.exe" /f /im TextInputHost.exe`
  Sleep 1000
FunctionEnd

Function StopControlCenter
  ; 覆盖桌面端 EXE 前必须释放 WebView 与主进程的文件句柄。
  ; 未运行时 taskkill 返回非零，不影响后续原位安装。
  !insertmacro RunHidden `"$SYSDIR\taskkill.exe" /f /im Shurufa.exe`
  Sleep 500
FunctionEnd

Function PrepareInPlaceUpdate
  Call StopControlCenter
  IfFileExists "$INSTDIR\shurufa-host.exe" 0 host_stop_done
  !insertmacro RunHidden `"$INSTDIR\shurufa-host.exe" stop`
host_stop_done:
  Call StopInputProcesses
  IfFileExists "$INSTDIR\${TSF_DLL_FILE}" 0 versioned_unregister_done
  !insertmacro RunHidden `"$RegistrationTool" /s /u "$INSTDIR\${TSF_DLL_FILE}"`
versioned_unregister_done:
  IfFileExists "$INSTDIR\shurufa_tsf.dll" 0 legacy_unregister_done
  !insertmacro RunHidden `"$RegistrationTool" /s /u "$INSTDIR\shurufa_tsf.dll"`
legacy_unregister_done:
  Sleep 1000
FunctionEnd

Section "安装 ${PRODUCT_NAME}" SEC_INSTALL
  Call PrepareInPlaceUpdate
  SetOutPath "$INSTDIR"
  FileOpen $InstallLog "$INSTDIR\install.log" w
  FileWriteWord $InstallLog 0xFEFF
  !insertmacro LogLine `开始安装 ${PRODUCT_NAME} ${PRODUCT_VERSION}`
  SetOverwrite off
  File /oname=${TSF_DLL_FILE} "..\target\release\shurufa_tsf.dll"
  SetOverwrite on
  File "..\target\release\shurufa-algo.exe"
  File "..\target\release\shurufa-host.exe"
  File "..\target\release\Shurufa.exe"
  File "activate-default-ime.ps1"
  File "register-host-startup.ps1"
  File "verify-install.ps1"
  File "..\third_party\librime\dist\lib\rime.dll"
  File "..\third_party\librime\dist\bin\rime_deployer.exe"
  SetOutPath "$INSTDIR\schemas"
  File /r "..\schemas\*.*"
  !insertmacro RunCritical `"$INSTDIR\rime_deployer.exe" --build "$INSTDIR\schemas" "$INSTDIR\schemas" "$INSTDIR\schemas\build"` `词典预构建`
  !insertmacro RunCritical `"$SYSDIR\icacls.exe" "$INSTDIR" /grant *S-1-15-2-1:(OI)(CI)(RX) /t /c` `授予输入法宿主读取权限`
  !insertmacro RunCritical `"$RegistrationTool" /s "$INSTDIR\${TSF_DLL_FILE}"` `注册 TSF 输入法`
  !insertmacro RunCritical `"$InputMethodTool" -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "$INSTDIR\activate-default-ime.ps1"` `设置默认输入法`
  !insertmacro RunCritical `"$InputMethodTool" -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "$INSTDIR\register-host-startup.ps1" -InstallDir "$INSTDIR"` `配置后台服务登录自启动`
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  SetOutPath "$INSTDIR"
  CreateDirectory "$SMPROGRAMS\Shurufa"
  CreateShortcut "$SMPROGRAMS\Shurufa\Shurufa.lnk" "$INSTDIR\Shurufa.exe" "" "$INSTDIR\Shurufa.exe" 0
  CreateShortcut "$DESKTOP\Shurufa.lnk" "$INSTDIR\Shurufa.exe" "" "$INSTDIR\Shurufa.exe" 0
  WriteRegStr HKLM "${PRODUCT_REGISTRY_KEY}" "DisplayName" "${PRODUCT_NAME}"
  WriteRegStr HKLM "${PRODUCT_REGISTRY_KEY}" "DisplayVersion" "${PRODUCT_VERSION}"
  WriteRegStr HKLM "${PRODUCT_REGISTRY_KEY}" "Publisher" "${PRODUCT_PUBLISHER}"
  WriteRegStr HKLM "${PRODUCT_REGISTRY_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKLM "${PRODUCT_REGISTRY_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${PRODUCT_REGISTRY_KEY}" "NoRepair" 1
  Exec `"$CtfmonTool"`
  ClearErrors
  ExecShell "open" "$INSTDIR\shurufa-host.exe" "supervise" SW_HIDE
  IfErrors 0 host_start_done
  !insertmacro LogLine `ExecShell 启动后台服务失败，改用 PowerShell 兜底启动`
  !insertmacro RunHidden `"$InputMethodTool" -NoProfile -NonInteractive -WindowStyle Hidden -Command "Start-Process -FilePath '$INSTDIR\shurufa-host.exe' -ArgumentList 'supervise' -WindowStyle Hidden"`
host_start_done:
  ; 终态验证：自启动键值与后台进程必须全部就绪。失败仅警告不回滚——
  ; 文件与注册均已完成，给出手工补救路径比中途中止更可恢复。
  !insertmacro RunHidden `"$InputMethodTool" -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "$INSTDIR\verify-install.ps1" -InstallDir "$INSTDIR"`
  !insertmacro LogLine `结果=$0 步骤=安装终态验证`
  StrCmp $0 "0" verify_done
  MessageBox MB_ICONEXCLAMATION `安装已完成，但未能确认后台服务就绪（结果：$0）。$\r$\n可手动运行："$INSTDIR\shurufa-host.exe" supervise$\r$\n或注销后重新登录。详见 $INSTDIR\install.log。` /SD IDOK
verify_done:
  !insertmacro LogLine `安装流程结束`
  FileClose $InstallLog
SectionEnd

Section "Uninstall"
  IfFileExists "$INSTDIR\shurufa-host.exe" 0 un_host_stop_done
  !insertmacro RunHidden `"$INSTDIR\shurufa-host.exe" stop`
un_host_stop_done:
  IfFileExists "$INSTDIR\register-host-startup.ps1" 0 un_startup_remove_done
  !insertmacro RunHidden `"$InputMethodTool" -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "$INSTDIR\register-host-startup.ps1" -Remove`
un_startup_remove_done:
  IfFileExists "$INSTDIR\activate-default-ime.ps1" 0 un_ime_clear_done
  !insertmacro RunHidden `"$InputMethodTool" -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "$INSTDIR\activate-default-ime.ps1" -Clear`
un_ime_clear_done:
  IfFileExists "$INSTDIR\${TSF_DLL_FILE}" 0 un_versioned_unregister_done
  !insertmacro RunHidden `"$RegistrationTool" /s /u "$INSTDIR\${TSF_DLL_FILE}"`
un_versioned_unregister_done:
  IfFileExists "$INSTDIR\shurufa_tsf.dll" 0 un_legacy_unregister_done
  !insertmacro RunHidden `"$RegistrationTool" /s /u "$INSTDIR\shurufa_tsf.dll"`
un_legacy_unregister_done:
  DeleteRegKey HKLM "${PRODUCT_REGISTRY_KEY}"
  Delete "$SMPROGRAMS\Shurufa\Shurufa.lnk"
  RMDir "$SMPROGRAMS\Shurufa"
  Delete "$DESKTOP\Shurufa.lnk"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir /r "$INSTDIR"
  IfErrors 0 uninstall_done
  MessageBox MB_ICONEXCLAMATION "部分文件仍被系统占用。请注销 Windows 后再次运行卸载程序。" /SD IDOK
uninstall_done:
SectionEnd
