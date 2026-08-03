Unicode true
RequestExecutionLevel admin

!include "MUI2.nsh"

!define PRODUCT_NAME "Shurufa 拼音"
!define PRODUCT_VERSION "0.4.0"
!define PRODUCT_PUBLISHER "Shurufa"
!define PRODUCT_REGISTRY_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Shurufa"

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

Var PreviousDirectory
Var AlternatePreviousDirectory
Var BackupCreated
Var RegistrationTool
Var InputMethodTool
Var InputMethodState
Var StartupState

!macro RunHidden Command
  nsExec::ExecToLog '${Command}'
  Pop $0
!macroend

Function .onInit
  SetShellVarContext all
  StrCpy $INSTDIR "$APPDATA\shurufa"
  StrCpy $PreviousDirectory "$APPDATA\shurufa.previous"
  StrCpy $AlternatePreviousDirectory "$APPDATA\shurufa.previous.next"
  StrCpy $BackupCreated "0"
  InitPluginsDir
  StrCpy $InputMethodState "$PLUGINSDIR\previous-input-tip.txt"
  StrCpy $StartupState "$PLUGINSDIR\previous-host-startup.json"
  IfFileExists "$WINDIR\Sysnative\regsvr32.exe" 0 use_default_registration_tool
  StrCpy $RegistrationTool "$WINDIR\Sysnative\regsvr32.exe"
  StrCpy $InputMethodTool "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
  Goto tools_ready
use_default_registration_tool:
  StrCpy $RegistrationTool "$SYSDIR\regsvr32.exe"
  StrCpy $InputMethodTool "$SYSDIR\WindowsPowerShell\v1.0\powershell.exe"
tools_ready:
FunctionEnd

Function un.onInit
  IfFileExists "$WINDIR\Sysnative\regsvr32.exe" 0 un_use_default_registration_tool
  StrCpy $RegistrationTool "$WINDIR\Sysnative\regsvr32.exe"
  StrCpy $InputMethodTool "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
  Goto un_tools_ready
un_use_default_registration_tool:
  StrCpy $RegistrationTool "$SYSDIR\regsvr32.exe"
  StrCpy $InputMethodTool "$SYSDIR\WindowsPowerShell\v1.0\powershell.exe"
un_tools_ready:
FunctionEnd

Function StopInputProcesses
  !insertmacro RunHidden '"$SYSDIR\taskkill.exe" /f /im ctfmon.exe'
  !insertmacro RunHidden '"$SYSDIR\taskkill.exe" /f /im TextInputHost.exe'
  Sleep 1000
FunctionEnd

Function BackupInstalledVersion
  ClearErrors
  IfFileExists "$PreviousDirectory\*.*" backup_exists
backup_current:
  IfFileExists "$INSTDIR\*.*" 0 backup_done
  IfFileExists "$INSTDIR\shurufa-host.exe" 0 +2
  !insertmacro RunHidden '"$INSTDIR\shurufa-host.exe" stop'
  IfFileExists "$INSTDIR\shurufa_tsf.dll" 0 +2
  !insertmacro RunHidden '"$RegistrationTool" /s /u "$INSTDIR\shurufa_tsf.dll"'
  Rename "$INSTDIR" "$PreviousDirectory"
  IfErrors backup_failed
  StrCpy $BackupCreated "1"
  Return

backup_exists:
  IfFileExists "$PreviousDirectory\shurufa-host.exe" 0 previous_host_stopped
  !insertmacro RunHidden '"$PreviousDirectory\shurufa-host.exe" stop'
  Sleep 1000
previous_host_stopped:
  Call StopInputProcesses
  IfFileExists "$INSTDIR\*.*" backup_conflict
  Rename "$PreviousDirectory" "$INSTDIR"
  IfErrors backup_restore_failed
  Goto backup_current

backup_conflict:
  MessageBox MB_ICONEXCLAMATION|MB_YESNO "检测到未处理的旧部署备份：$PreviousDirectory$\r$\n当前安装目录也存在。删除旧备份并继续安装吗？\r$\n选择“否”将取消安装并保留备份。" IDYES backup_remove_stale IDNO backup_cancel
  Goto backup_cancel

backup_remove_stale:
  IfFileExists "$INSTDIR\shurufa-host.exe" 0 +2
  !insertmacro RunHidden '"$INSTDIR\shurufa-host.exe" stop'
  Sleep 1000
  Call StopInputProcesses
  ClearErrors
  RMDir /r "$PreviousDirectory"
  IfErrors backup_remove_failed
  Goto backup_current

backup_remove_failed:
  MessageBox MB_ICONEXCLAMATION|MB_YESNO "旧部署备份仍被系统占用：$PreviousDirectory$\r$\n是否保留该备份并使用新的回滚目录继续安装？重启 Windows 后系统会清理旧备份。\r$\n选择“否”将取消安装。" IDYES backup_use_alternate IDNO backup_cancel
  Goto backup_cancel

backup_use_alternate:
  ClearErrors
  RMDir /r /REBOOTOK "$PreviousDirectory"
  ClearErrors
  IfFileExists "$AlternatePreviousDirectory\*.*" 0 alternate_ready
  MessageBox MB_ICONSTOP "新的回滚目录也已存在：$AlternatePreviousDirectory$\r$\n请先完成上一次安装或重启 Windows 后再试。"
  Goto backup_cancel
alternate_ready:
  StrCpy $PreviousDirectory "$AlternatePreviousDirectory"
  Goto backup_current

backup_restore_failed:
  MessageBox MB_ICONSTOP "无法恢复旧部署备份到：$INSTDIR$\r$\n请检查目录权限后重试。"
  Goto backup_cancel

backup_cancel:
  SetErrors
  Return

backup_failed:
  IfFileExists "$INSTDIR\shurufa_tsf.dll" 0 +2
  !insertmacro RunHidden '"$RegistrationTool" /s "$INSTDIR\shurufa_tsf.dll"'
  MessageBox MB_ICONSTOP "无法替换当前部署，可能仍被输入法进程占用。请注销 Windows 后重试。"
  SetErrors
  Return

backup_done:
  Return
FunctionEnd

Function RestorePreviousVersion
  IfFileExists "$INSTDIR\register-host-startup.ps1" 0 +2
  !insertmacro RunHidden '"$InputMethodTool" -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "$INSTDIR\register-host-startup.ps1" -Restore -StatePath "$StartupState"'
  IfFileExists "$INSTDIR\activate-default-ime.ps1" 0 +2
  !insertmacro RunHidden '"$InputMethodTool" -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "$INSTDIR\activate-default-ime.ps1" -Restore -StatePath "$InputMethodState"'
  RMDir /r "$INSTDIR"
  StrCmp $BackupCreated "1" 0 restore_done
  Rename "$PreviousDirectory" "$INSTDIR"
  IfErrors restore_failed
  IfFileExists "$INSTDIR\shurufa_tsf.dll" 0 restore_done
  !insertmacro RunHidden '"$RegistrationTool" /s "$INSTDIR\shurufa_tsf.dll"'
  Goto restore_done

restore_failed:
  MessageBox MB_ICONSTOP "新版本安装失败，且无法自动恢复旧版本。旧文件仍位于：$PreviousDirectory"
restore_done:
FunctionEnd

Section "安装 ${PRODUCT_NAME}" SEC_INSTALL
  Call BackupInstalledVersion
  IfErrors install_cancelled
  SetOutPath "$INSTDIR"
  File "..\target\release\shurufa_tsf.dll"
  File "..\target\release\shurufa-algo.exe"
  File "..\target\release\shurufa-host.exe"
  File "..\target\release\Shurufa.exe"
  File "activate-default-ime.ps1"
  File "register-host-startup.ps1"
  File "..\third_party\librime\dist\lib\rime.dll"
  File "..\third_party\librime\dist\bin\rime_deployer.exe"
  SetOutPath "$INSTDIR\schemas"
  File /r "..\schemas\*.*"
  !insertmacro RunHidden '"$INSTDIR\rime_deployer.exe" --build "$INSTDIR\schemas" "$INSTDIR\schemas" "$INSTDIR\schemas\build"'
  IntCmp $0 0 +3
  MessageBox MB_ICONSTOP "词典预构建失败，已恢复旧版本。"
  Goto install_failed
  !insertmacro RunHidden '"$SYSDIR\icacls.exe" "$INSTDIR" /grant *S-1-15-2-1:(OI)(CI)(RX) /t /c'
  IntCmp $0 0 +3
  MessageBox MB_ICONSTOP "无法授予输入法宿主读取权限，已恢复旧版本。"
  Goto install_failed
  !insertmacro RunHidden '"$RegistrationTool" /s "$INSTDIR\shurufa_tsf.dll"'
  IntCmp $0 0 +3
  MessageBox MB_ICONSTOP "TSF 注册失败，已恢复旧版本。"
  Goto install_failed
  !insertmacro RunHidden '"$InputMethodTool" -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "$INSTDIR\activate-default-ime.ps1" -StatePath "$InputMethodState"'
  IntCmp $0 0 +3
  MessageBox MB_ICONSTOP "无法将 Shurufa 拼音设为默认输入法，已恢复旧版本。"
  Goto install_failed
  !insertmacro RunHidden '"$InputMethodTool" -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "$INSTDIR\register-host-startup.ps1" -InstallDir "$INSTDIR" -StatePath "$StartupState"'
  IntCmp $0 0 +3
  MessageBox MB_ICONSTOP "无法配置 Shurufa 后台服务的登录启动，已恢复旧版本。"
  Goto install_failed
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateDirectory "$SMPROGRAMS\Shurufa"
  CreateShortcut "$SMPROGRAMS\Shurufa\Shurufa.lnk" "$INSTDIR\Shurufa.exe"
  CreateShortcut "$DESKTOP\Shurufa.lnk" "$INSTDIR\Shurufa.exe"
  WriteRegStr HKLM "${PRODUCT_REGISTRY_KEY}" "DisplayName" "${PRODUCT_NAME}"
  WriteRegStr HKLM "${PRODUCT_REGISTRY_KEY}" "DisplayVersion" "${PRODUCT_VERSION}"
  WriteRegStr HKLM "${PRODUCT_REGISTRY_KEY}" "Publisher" "${PRODUCT_PUBLISHER}"
  WriteRegStr HKLM "${PRODUCT_REGISTRY_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKLM "${PRODUCT_REGISTRY_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${PRODUCT_REGISTRY_KEY}" "NoRepair" 1
  Call StopInputProcesses
  ClearErrors
  RMDir /r /REBOOTOK "$PreviousDirectory"
  IfErrors backup_cleanup_warning backup_cleanup_done
backup_cleanup_warning:
  MessageBox MB_ICONEXCLAMATION "安装已完成，但旧备份仍被系统占用：$PreviousDirectory$\r$\n重启 Windows 后该备份会自动清理。"
backup_cleanup_done:
  Exec '"$SYSDIR\ctfmon.exe"'
  ExecShell "open" "$INSTDIR\shurufa-host.exe" "supervise" SW_HIDE
  Goto install_done

install_failed:
  Call RestorePreviousVersion
  Abort
install_cancelled:
  Abort
install_done:
SectionEnd

Section "Uninstall"
  IfFileExists "$INSTDIR\shurufa-host.exe" 0 +2
  !insertmacro RunHidden '"$INSTDIR\shurufa-host.exe" stop'
  IfFileExists "$INSTDIR\register-host-startup.ps1" 0 +2
  !insertmacro RunHidden '"$InputMethodTool" -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "$INSTDIR\register-host-startup.ps1" -Remove'
  IfFileExists "$INSTDIR\shurufa_tsf.dll" 0 +2
  !insertmacro RunHidden '"$RegistrationTool" /s /u "$INSTDIR\shurufa_tsf.dll"'
  DeleteRegKey HKLM "${PRODUCT_REGISTRY_KEY}"
  Delete "$SMPROGRAMS\Shurufa\Shurufa.lnk"
  RMDir "$SMPROGRAMS\Shurufa"
  Delete "$DESKTOP\Shurufa.lnk"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir /r "$INSTDIR"
  IfErrors 0 uninstall_done
  MessageBox MB_ICONEXCLAMATION "部分文件仍被系统占用。请注销 Windows 后再次运行卸载程序。"
uninstall_done:
SectionEnd
