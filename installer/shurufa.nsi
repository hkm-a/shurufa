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
Var InputMethodState
Var StartupState

!macro RunHidden Command
  nsExec::ExecToLog '${Command}'
  Pop $0
!macroend

Function .onInit
  SetShellVarContext all
  StrCpy $INSTDIR "$APPDATA\shurufa"
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

Function PrepareInPlaceUpdate
  IfFileExists "$INSTDIR\shurufa-host.exe" 0 +2
  !insertmacro RunHidden '"$INSTDIR\shurufa-host.exe" stop'
  Call StopInputProcesses
  IfFileExists "$INSTDIR\${TSF_DLL_FILE}" 0 +2
  !insertmacro RunHidden '"$RegistrationTool" /s /u "$INSTDIR\${TSF_DLL_FILE}"'
  IfFileExists "$INSTDIR\shurufa_tsf.dll" 0 +2
  !insertmacro RunHidden '"$RegistrationTool" /s /u "$INSTDIR\shurufa_tsf.dll"'
  Sleep 1000
FunctionEnd

Section "安装 ${PRODUCT_NAME}" SEC_INSTALL
  Call PrepareInPlaceUpdate
  SetOutPath "$INSTDIR"
  SetOverwrite off
  File /oname=${TSF_DLL_FILE} "..\target\release\shurufa_tsf.dll"
  SetOverwrite on
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
  MessageBox MB_ICONSTOP "词典预构建失败。现有安装目录不会被删除，请关闭占用程序后重试。"
  Goto install_failed
  !insertmacro RunHidden '"$SYSDIR\icacls.exe" "$INSTDIR" /grant *S-1-15-2-1:(OI)(CI)(RX) /t /c'
  IntCmp $0 0 +3
  MessageBox MB_ICONSTOP "无法授予输入法宿主读取权限。现有安装目录不会被删除，请关闭占用程序后重试。"
  Goto install_failed
  !insertmacro RunHidden '"$RegistrationTool" /s "$INSTDIR\${TSF_DLL_FILE}"'
  IntCmp $0 0 +3
  MessageBox MB_ICONSTOP "TSF 注册失败。现有安装目录不会被删除，请关闭占用程序后重试。"
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
  Exec '"$SYSDIR\ctfmon.exe"'
  ExecShell "open" "$INSTDIR\shurufa-host.exe" "supervise" SW_HIDE
  Goto install_done

install_failed:
  Abort
install_done:
SectionEnd

Section "Uninstall"
  IfFileExists "$INSTDIR\shurufa-host.exe" 0 +2
  !insertmacro RunHidden '"$INSTDIR\shurufa-host.exe" stop'
  IfFileExists "$INSTDIR\register-host-startup.ps1" 0 +2
  !insertmacro RunHidden '"$InputMethodTool" -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "$INSTDIR\register-host-startup.ps1" -Remove'
  IfFileExists "$INSTDIR\${TSF_DLL_FILE}" 0 +2
  !insertmacro RunHidden '"$RegistrationTool" /s /u "$INSTDIR\${TSF_DLL_FILE}"'
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
