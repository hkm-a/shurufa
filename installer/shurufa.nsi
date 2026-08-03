Unicode true
RequestExecutionLevel admin

!define PRODUCT_NAME "Shurufa 拼音"
!define PRODUCT_VERSION "0.1.0"
!define PRODUCT_PUBLISHER "Shurufa"
!define PRODUCT_REGISTRY_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Shurufa"

Name "${PRODUCT_NAME} ${PRODUCT_VERSION}"
OutFile "..\dist\Shurufa-Setup.exe"
InstallDir "$APPDATA\shurufa"
ShowInstDetails show
ShowUninstDetails show

Var PreviousDirectory
Var BackupCreated
Var RegistrationTool
Var InputMethodTool
Var InputMethodState
Var StartupState

Function .onInit
  SetShellVarContext all
  StrCpy $INSTDIR "$APPDATA\shurufa"
  StrCpy $PreviousDirectory "$APPDATA\shurufa.previous"
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

Function BackupInstalledVersion
  ClearErrors
  IfFileExists "$PreviousDirectory\*.*" backup_exists
  IfFileExists "$INSTDIR\*.*" 0 backup_done
  IfFileExists "$INSTDIR\shurufa-host.exe" 0 +2
  ExecWait '"$INSTDIR\shurufa-host.exe" stop' $0
  IfFileExists "$INSTDIR\shurufa_tsf.dll" 0 +2
  ExecWait '"$RegistrationTool" /s /u "$INSTDIR\shurufa_tsf.dll"' $0
  Rename "$INSTDIR" "$PreviousDirectory"
  IfErrors backup_failed
  StrCpy $BackupCreated "1"
  Return

backup_exists:
  MessageBox MB_ICONSTOP "检测到未处理的旧部署备份：$PreviousDirectory$\r$\n请先确认或移除该目录，再重新安装。"
  SetErrors
  Return

backup_failed:
  IfFileExists "$INSTDIR\shurufa_tsf.dll" 0 +2
  ExecWait '"$RegistrationTool" /s "$INSTDIR\shurufa_tsf.dll"' $0
  MessageBox MB_ICONSTOP "无法替换当前部署，可能仍被输入法进程占用。请注销 Windows 后重试。"
  SetErrors
  Return

backup_done:
  Return
FunctionEnd

Function RestorePreviousVersion
  IfFileExists "$INSTDIR\register-host-startup.ps1" 0 +2
  ExecWait '"$InputMethodTool" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$INSTDIR\register-host-startup.ps1" -Restore -StatePath "$StartupState"' $0
  IfFileExists "$INSTDIR\activate-default-ime.ps1" 0 +2
  ExecWait '"$InputMethodTool" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$INSTDIR\activate-default-ime.ps1" -Restore -StatePath "$InputMethodState"' $0
  RMDir /r "$INSTDIR"
  StrCmp $BackupCreated "1" 0 restore_done
  Rename "$PreviousDirectory" "$INSTDIR"
  IfErrors restore_failed
  IfFileExists "$INSTDIR\shurufa_tsf.dll" 0 restore_done
  ExecWait '"$RegistrationTool" /s "$INSTDIR\shurufa_tsf.dll"' $0
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
  File "..\target\release\shurufa-settings.exe"
  File "activate-default-ime.ps1"
  File "register-host-startup.ps1"
  File "..\third_party\librime\dist\lib\rime.dll"
  File "..\third_party\librime\dist\bin\rime_deployer.exe"
  SetOutPath "$INSTDIR\schemas"
  File /r "..\schemas\*.*"
  ExecWait '"$INSTDIR\rime_deployer.exe" --build "$INSTDIR\schemas" "$INSTDIR\schemas" "$INSTDIR\schemas\build"' $0
  IntCmp $0 0 +3
  MessageBox MB_ICONSTOP "词典预构建失败，已恢复旧版本。"
  Goto install_failed
  ExecWait '"$SYSDIR\icacls.exe" "$INSTDIR" /grant *S-1-15-2-1:(OI)(CI)(RX) /t /c' $0
  IntCmp $0 0 +3
  MessageBox MB_ICONSTOP "无法授予输入法宿主读取权限，已恢复旧版本。"
  Goto install_failed
  ExecWait '"$RegistrationTool" /s "$INSTDIR\shurufa_tsf.dll"' $0
  IntCmp $0 0 +3
  MessageBox MB_ICONSTOP "TSF 注册失败，已恢复旧版本。"
  Goto install_failed
  ExecWait '"$InputMethodTool" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$INSTDIR\activate-default-ime.ps1" -StatePath "$InputMethodState"' $0
  IntCmp $0 0 +3
  MessageBox MB_ICONSTOP "无法将 Shurufa 拼音设为默认输入法，已恢复旧版本。"
  Goto install_failed
  ExecWait '"$InputMethodTool" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$INSTDIR\register-host-startup.ps1" -InstallDir "$INSTDIR" -StatePath "$StartupState"' $0
  IntCmp $0 0 +3
  MessageBox MB_ICONSTOP "无法配置 Shurufa 后台服务的登录启动，已恢复旧版本。"
  Goto install_failed
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKLM "${PRODUCT_REGISTRY_KEY}" "DisplayName" "${PRODUCT_NAME}"
  WriteRegStr HKLM "${PRODUCT_REGISTRY_KEY}" "DisplayVersion" "${PRODUCT_VERSION}"
  WriteRegStr HKLM "${PRODUCT_REGISTRY_KEY}" "Publisher" "${PRODUCT_PUBLISHER}"
  WriteRegStr HKLM "${PRODUCT_REGISTRY_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKLM "${PRODUCT_REGISTRY_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${PRODUCT_REGISTRY_KEY}" "NoRepair" 1
  Exec '"$INSTDIR\shurufa-host.exe" supervise'
  MessageBox MB_OK "安装完成。Shurufa 拼音已设为默认输入法。"
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
  ExecWait '"$INSTDIR\shurufa-host.exe" stop' $0
  IfFileExists "$INSTDIR\register-host-startup.ps1" 0 +2
  ExecWait '"$InputMethodTool" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$INSTDIR\register-host-startup.ps1" -Remove' $0
  IfFileExists "$INSTDIR\shurufa_tsf.dll" 0 +2
  ExecWait '"$RegistrationTool" /s /u "$INSTDIR\shurufa_tsf.dll"' $0
  DeleteRegKey HKLM "${PRODUCT_REGISTRY_KEY}"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir /r "$INSTDIR"
  IfErrors 0 uninstall_done
  MessageBox MB_ICONEXCLAMATION "部分文件仍被系统占用。请注销 Windows 后再次运行卸载程序。"
uninstall_done:
SectionEnd
