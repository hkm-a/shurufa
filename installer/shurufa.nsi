Unicode true
RequestExecutionLevel admin

!include "MUI2.nsh"

!define PRODUCT_NAME "Shurufa 拼音"
!define PRODUCT_VERSION "0.4.1"
!define PRODUCT_PUBLISHER "Shurufa"
!define PRODUCT_REGISTRY_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Shurufa"
!define TSF_DLL_FILE "shurufa_tsf-${PRODUCT_VERSION}.dll"

Name "${PRODUCT_NAME} ${PRODUCT_VERSION}"
OutFile "..\dist\Shurufa-Setup-${PRODUCT_VERSION}.exe"
; SetShellVarContext all 之后 $APPDATA 被重定向到 C:\ProgramData（而非某个用户
; 的 Roaming），InstallDir 只是默认值，真正的覆盖发生在 .onInit。该隐式重定向
; 必须与 scripts/install.ps1 默认的 $env:ProgramData\shurufa 保持一致。
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
  SetShellVarContext all
  StrCpy $INSTDIR "$APPDATA\shurufa"
  IfFileExists "$WINDIR\Sysnative\regsvr32.exe" 0 un_use_default_tools
  StrCpy $RegistrationTool "$WINDIR\Sysnative\regsvr32.exe"
  StrCpy $InputMethodTool "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
  Goto un_tools_ready
un_use_default_tools:
  StrCpy $RegistrationTool "$SYSDIR\regsvr32.exe"
  StrCpy $InputMethodTool "$SYSDIR\WindowsPowerShell\v1.0\powershell.exe"
un_tools_ready:
  ; 卸载必须有日志，否则失败后用户与排障均无线索（曾"静默失败"数日无人知）。
  ; 写到 $TEMP 子目录保证 RMDir /r $INSTDIR 成功后日志仍可查阅。
  CreateDirectory "$TEMP\Shurufa-Setup"
  FileOpen $InstallLog "$TEMP\Shurufa-Setup\uninstall.log" w
  FileWriteWord $InstallLog 0xFEFF
  !insertmacro LogLine `开始卸载 ${PRODUCT_NAME} 自 $INSTDIR`
FunctionEnd

; 卸载前先停宿主进程（ctfmon.exe / TextInputHost.exe / Shurufa.exe）
; 卸载下导出为 un.* 函数，便于与安装侧 StopInputProcesses 并存。
Function un.PreStopInputProcesses
  !insertmacro RunHidden `"$SYSDIR\taskkill.exe" /f /im Shurufa.exe`
  !insertmacro RunHidden `"$SYSDIR\taskkill.exe" /f /im ctfmon.exe`
  !insertmacro RunHidden `"$SYSDIR\taskkill.exe" /f /im TextInputHost.exe`
  Sleep 1000
FunctionEnd

; 卸载时删除且反注册所有仍存在的 shurufa_tsf-*.dll（覆盖跨版本升级残留场景）。
; FindFirst/FindNext 枚举 + goto 循环；不依赖 LogicLib。
Function un.TryUnregisterAllTsfDlls
  Push $R0
  Push $R1
  FindFirst $R0 $R1 "$INSTDIR\shurufa_tsf-*.dll"
un_tsf_enum_loop:
  StrCmp $R1 "" un_tsf_enum_done
  !insertmacro LogLine `反注册残留 TSF $R1`
  !insertmacro RunHidden `"$RegistrationTool" /s /u "$INSTDIR\$R1"`
  FindNext $R0 $R1
  Goto un_tsf_enum_loop
un_tsf_enum_done:
  FindClose $R0
  Pop $R1
  Pop $R0
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
  ; 先杀掉占用 DLL 的进程，否则后续反注册与删除会因 locks 静默失败。
  Call un.PreStopInputProcesses

  ; 关键步骤：若不起 shurufa-host 则中止，否则后续残留进程会重新写注册。
  IfFileExists "$INSTDIR\shurufa-host.exe" 0 un_host_stop_done
  nsExec::ExecToLog `"$INSTDIR\shurufa-host.exe" stop`
  Pop $0
  !insertmacro LogLine `结果=$0 步骤=停止后台服务`
  StrCmp $0 "0" un_host_stop_done
  !insertmacro LogLine `后台服务 stop 未返回 0，但继续（进程可能本就不在）`
un_host_stop_done:

  IfFileExists "$INSTDIR\register-host-startup.ps1" 0 un_startup_remove_done
  nsExec::ExecToLog `"$InputMethodTool" -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "$INSTDIR\register-host-startup.ps1" -Remove`
  Pop $0
  !insertmacro LogLine `结果=$0 步骤=移除登录自启动`
un_startup_remove_done:

  IfFileExists "$INSTDIR\activate-default-ime.ps1" 0 un_ime_clear_done
  nsExec::ExecToLog `"$InputMethodTool" -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "$INSTDIR\activate-default-ime.ps1" -Clear`
  Pop $0
  !insertmacro LogLine `结果=$0 步骤=清除默认输入法`
un_ime_clear_done:

  ; 反注册当前版本 TSF
  IfFileExists "$INSTDIR\${TSF_DLL_FILE}" 0 un_versioned_unregister_done
  nsExec::ExecToLog `"$RegistrationTool" /s /u "$INSTDIR\${TSF_DLL_FILE}"`
  Pop $0
  !insertmacro LogLine `结果=$0 步骤=反注册 ${TSF_DLL_FILE}`
un_versioned_unregister_done:

  ; 兼容旧文件名
  IfFileExists "$INSTDIR\shurufa_tsf.dll" 0 un_legacy_unregister_done
  nsExec::ExecToLog `"$RegistrationTool" /s /u "$INSTDIR\shurufa_tsf.dll"`
  Pop $0
  !insertmacro LogLine `结果=$0 步骤=反注册 shurufa_tsf.dll`
un_legacy_unregister_done:

  ; 枚举跨版本残留（shurufa_tsf-*.dll）
  Call un.TryUnregisterAllTsfDlls

  DeleteRegKey HKLM "${PRODUCT_REGISTRY_KEY}"
  Delete "$SMPROGRAMS\Shurufa\Shurufa.lnk"
  RMDir "$SMPROGRAMS\Shurufa"
  Delete "$DESKTOP\Shurufa.lnk"
  Delete "$INSTDIR\Uninstall.exe"

  ; 不能再删 $TEMP\Shurufa-Setup\uninstall.log 之上，RMDir /r $INSTDIR 不会波及它。
  RMDir /r "$INSTDIR"
  IfErrors 0 un_rmdir_ok
  ; 删除失败（常被 ctfmon/TextInputHost 持有 DLL 导致）→ 弹醒目警告而不是静默完成。
  !insertmacro LogLine `RMDir /r $INSTDIR 失败，文件可能被输入法宿主进程占用`
  MessageBox MB_ICONEXCLAMATION "卸载遇到文件占用。$\r$\n已尽力反注册服务与注册表，但 $INSTDIR 中可能有残余 DLL。$\r$\n请重启 Windows 后再次运行卸载程序，或手动删除该目录。详见 $TEMP\Shurufa-Setup\uninstall.log。" /SD IDOK
  Goto uninstall_done
un_rmdir_ok:
  ; 复核：如果注册表/关键文件仍在，告警而不是假装成功。
  IfFileExists "$INSTDIR\shurufa-host.exe" 0 un_check_registry
  !insertmacro LogLine `复核：shurufa-host.exe 仍存在`
  MessageBox MB_ICONEXCLAMATION "卸载后检测到残留文件。请重启 Windows 后手动删除 $INSTDIR。" /SD IDOK
un_check_registry:
  ReadRegStr $0 HKLM "${PRODUCT_REGISTRY_KEY}" "UninstallString"
  StrCmp $0 "" uninstall_done
  !insertmacro LogLine `复核：注册表 ${PRODUCT_REGISTRY_KEY} 仍存在`
  DeleteRegKey HKLM "${PRODUCT_REGISTRY_KEY}"
uninstall_done:
  !insertmacro LogLine `卸载流程结束`
  FileClose $InstallLog
SectionEnd
