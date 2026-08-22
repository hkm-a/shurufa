; FOX 输入法 NSIS 安装脚本（阶段4第5项：安装器回 NSIS）。
; 逻辑移植自 platforms/windows-installer/src/engine.rs（Tauri 自研安装器）：
;   停旧进程 → 反注册/清理旧安装 → 写入 payload（TSF DLL 被占用回退唯一文件名）
;   → rime 词典预构建 → icacls 授予 AppContainer → regsvr32 注册 TSF →
;   清理孤儿 TSF DLL → 自启动 → 快捷方式 → 卸载注册表 → IFEO 高优先级 →
;   启动宿主（schtasks 降权）→ 终态验证。
; 超出 NSIS 框架能力的两处（IFEO、TSF DLL 回退）以自定义段承载。
; 构建：makensis -DFOX_VERSION=<version> installer\shurufa.nsi
;   （payload 取 target/release 与 schemas/，需先跑 build-installer.ps1 的构建步骤）

!ifndef FOX_VERSION
  !define FOX_VERSION "0.0.0"
!endif

Unicode true
ManifestDPIAware true
RequestExecutionLevel admin

Name "FOX输入法 ${FOX_VERSION}"
OutFile "..\dist\FOX-Setup-${FOX_VERSION}.exe"
InstallDir "C:\Program Files\FOX"
; 升级场景：优先沿用上次的安装目录
InstallDirRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\FOX输入法" "InstallLocation"

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "x64.nsh"

!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\FOX输入法"
!define IFEO_ROOT "Software\Microsoft\Windows NT\CurrentVersion\Image File Execution Options"
; 输入法 TIP（与 Deploy-Shurufa.ps1 的 $script:ShurufaInputTip 保持一致，SSOT）
!define DEFAULT_IME_TIP "0804:{8A5C1B49-3D2E-4F7A-9C61-0B7E2D5A9F13}{C4E9D2A7-6B31-4A58-8F0D-1E9A7C3B5D26}"

!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\Shurufa.exe"
!define MUI_FINISHPAGE_RUN_NOTCHECKED
!define MUI_FINISHPAGE_RUN_TEXT "立即运行 FOX 控制中心"
!define MUI_FINISHPAGE_RUN_FUNCTION "LaunchFoxAsUser"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "SimpChinese"

; 本次实际注册的 TSF DLL 文件名（可能因文件占用回退为带时间戳的唯一名）
Var TsfDll

;──────────────────────── 工具函数 ────────────────────────

; 多轮「杀 → WMI 兜底」（engine.rs stop_process 的 NSIS 版）：
; taskkill 对挂起进程可能报"无实例"但进程仍存活并锁住文件，
; 每轮补一发 WMI Terminate，多轮之间留句柄释放时间。
Function StopProc
  ; $0 = 进程名
  StrCpy $1 0
  ${DoWhile} $1 < 3
    nsExec::ExecToLog 'taskkill /f /im $0'
    nsExec::ExecToLog 'powershell -NoProfile -NonInteractive -WindowStyle Hidden -Command "Get-CimInstance Win32_Process -Filter \"Name='''$0'''\" | ForEach-Object { Invoke-CimMethod -InputObject $$_. -MethodName Terminate | Out-Null }"'
    Sleep 1200
    IntOp $1 $1 + 1
  ${Loop}
FunctionEnd

; 以普通用户身份启动程序（engine.rs launch_as_user 的 NSIS 版）。
; 绝不能从提权安装器直接 Exec——宿主链提权会让算法服务的 IPC 管道
; 拒绝普通应用（err=5），输入法整体失效（2026-08-14 实机复现）。
; schtasks 一次性任务使用交互用户的受限令牌，跑完即删。
; $0 = 目标 exe 完整路径，$1 = 参数（可为空）
Function LaunchAsUser
  StrCpy $2 '"$0"'
  ${If} $1 != ""
    StrCpy $2 '$2 $1'
  ${EndIf}
  nsExec::ExecToLog 'schtasks /Create /TN FOXUserLaunch /TR "$2" /SC ONCE /ST 00:00 /RL LIMITED /F'
  nsExec::ExecToLog 'schtasks /Run /TN FOXUserLaunch'
  nsExec::ExecToLog 'schtasks /Delete /TN FOXUserLaunch /F'
FunctionEnd

Function LaunchFoxAsUser
  StrCpy $0 "$INSTDIR\Shurufa.exe"
  StrCpy $1 ""
  Call LaunchAsUser
FunctionEnd

; 反注册 $INSTDIR 下所有 shurufa_tsf*.dll（当前版本名 + 历史残留）
Function UnregisterTsf
  ; 主名与无版本名
  IfFileExists "$INSTDIR\shurufa_tsf-${FOX_VERSION}.dll" 0 +2
    nsExec::ExecToLog '"$SYSDIR\regsvr32.exe" /s /u "$INSTDIR\shurufa_tsf-${FOX_VERSION}.dll"'
  IfFileExists "$INSTDIR\shurufa_tsf.dll" 0 +2
    nsExec::ExecToLog '"$SYSDIR\regsvr32.exe" /s /u "$INSTDIR\shurufa_tsf.dll"'
  ; 枚举跨版本残留
  FindFirst $3 $2 "$INSTDIR\shurufa_tsf-*.dll"
  ${DoWhile} $2 != ""
    nsExec::ExecToLog '"$SYSDIR\regsvr32.exe" /s /u "$INSTDIR\$2"'
    FindNext $3 $2
  ${Loop}
  FindClose $3
FunctionEnd

; 删除旧安装目录（一台机器只保留一个安装）。
Function CleanLegacyDir
  ; $0 = 待清理目录（含 shurufa-host.exe 才算本产品安装目录）
  IfFileExists "$0\shurufa-clipd.exe" 0 done
  Push $INSTDIR
  StrCpy $INSTDIR $0
  Call UnregisterTsf
  Pop $INSTDIR
  RMDir /r "$0"
  done:
FunctionEnd

;──────────────────────── 安装 ────────────────────────

Section "FOX输入法（核心）" SecCore
  SectionIn RO
  SetOutPath "$INSTDIR"

  DetailPrint "步骤 1/10 停旧进程与反注册旧 TSF"
  StrCpy $0 "Shurufa.exe"
  Call StopProc
  StrCpy $0 "shurufa-ui.exe"
  Call StopProc
  StrCpy $0 "shurufa-clipd.exe"
  Call StopProc
  StrCpy $0 "shurufa-algo.exe"
  Call StopProc
  StrCpy $0 "ctfmon.exe"
  Call StopProc
  StrCpy $0 "TextInputHost.exe"
  Call StopProc
  Sleep 1500
  Call UnregisterTsf
  ; 清理旧版目录（%ProgramDATA%\shurufa 与卸载注册表记录的异路径旧安装）
  ReadRegStr $0 HKLM "${UNINST_KEY}" "InstallLocation"
  ${If} $0 != ""
  ${AndIf} $0 != $INSTDIR
    Push $0
    Call CleanLegacyDir
    Pop $0
  ${EndIf}
  ; %ProgramData%\shurufa：SetShellVarContext all 下 $APPDATA 即 CSIDL_COMMON_APPDATA
  SetShellVarContext all
  StrCpy $0 "$APPDATA\shurufa"
  SetShellVarContext current
  Push $0
  Call CleanLegacyDir
  Pop $0

  DetailPrint "步骤 2/10 写入程序文件"
  File "..\target\release\shurufa-algo.exe"
  File "..\target\release\shurufa-clipd.exe"
  File "..\target\release\shurufa-ui.exe"
  File "..\target\release\shurufa-ctl.exe"
  File "..\target\release\Shurufa.exe"
  File "..\third_party\librime\dist\lib\rime.dll"
  File "..\third_party\librime\dist\bin\rime_deployer.exe"
  File "activate-default-ime.ps1"
  File "register-host-startup.ps1"
  File "verify-install.ps1"
  File "Deploy-Shurufa.ps1"
  File /r "..\schemas"

  ; TSF DLL：被已注册旧版锁住（所有文本输入进程都会加载它，杀不光）时，
  ; 重试 6 次（每轮补杀 host/algo），仍失败则回退带时间戳的唯一文件名。
  StrCpy $R0 "shurufa_tsf-${FOX_VERSION}.dll"
  StrCpy $4 0
  ${Do}
    ClearErrors
    File /nonfatal /oname=$R0 "..\target\release\shurufa_tsf.dll"
    ${IfNot} ${Errors}
      ${ExitDo}
    ${EndIf}
    IntOp $4 $4 + 1
    ${If} $4 >= 6
      ; 回退唯一文件名（Unix 秒）：GetTickCount 足够唯一（同毫秒重装可忽略）
      System::Call 'kernel32::GetTickCount()i .r5'
      StrCpy $R0 "shurufa_tsf-${FOX_VERSION}-$5.dll"
      ClearErrors
      File /nonfatal /oname=$R0 "..\target\release\shurufa_tsf.dll"
      ${If} ${Errors}
        MessageBox MB_OK|MB_ICONSTOP "写入 $INSTDIR\$R0 失败：文件被占用。请重启后再试。"
        Abort
      ${EndIf}
      DetailPrint "TSF DLL 被旧进程锁定，回退唯一文件名：$R0"
      ${ExitDo}
    ${EndIf}
    DetailPrint "TSF DLL 写入失败（第 $4 次），重试…"
    StrCpy $0 "shurufa-algo.exe"
    Call StopProc
    StrCpy $0 "shurufa-clipd.exe"
    Call StopProc
    Sleep 800
  ${Loop}
  StrCpy $TsfDll $R0

  DetailPrint "步骤 3/10 rime 词典预构建"
  nsExec::ExecToLog '"$INSTDIR\rime_deployer.exe" --build "$INSTDIR\schemas" "$INSTDIR\schemas" "$INSTDIR\schemas\build"'
  Pop $0
  ${If} $0 != 0
    MessageBox MB_OK|MB_ICONSTOP "词典预构建失败（退出码 $0）。"
    Abort
  ${EndIf}

  DetailPrint "步骤 4/10 配置 AppContainer 读取权限"
  nsExec::ExecToLog '"$SYSDIR\icacls.exe" "$INSTDIR" /grant *S-1-15-2-1:(OI)(CI)(RX) /t /c'
  Pop $0
  ${If} $0 != 0
    MessageBox MB_OK|MB_ICONSTOP "授予权限失败（退出码 $0）。"
    Abort
  ${EndIf}

  DetailPrint "步骤 5/10 注册输入法（$TsfDll）"
  nsExec::ExecToLog '"$SYSDIR\regsvr32.exe" /s "$INSTDIR\$TsfDll"'
  Pop $0
  ${If} $0 != 0
    MessageBox MB_OK|MB_ICONSTOP "注册 TSF 输入法失败（退出码 $0）。"
    Abort
  ${EndIf}

  ; 清理孤儿 TSF DLL：只保留本次注册的文件，删掉历次安装累积的残留
  FindFirst $3 $2 "$INSTDIR\shurufa_tsf*.dll"
  ${DoWhile} $2 != ""
    ${If} $2 != "$TsfDll"
      Delete "$INSTDIR\$2"
    ${EndIf}
    FindNext $3 $2
  ${Loop}
  FindClose $3

  DetailPrint "步骤 6/10 配置自启动"
  nsExec::ExecToLog 'powershell -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "$INSTDIR\register-host-startup.ps1" -InstallDir "$INSTDIR"'
  Pop $0
  ${If} $0 != 0
    MessageBox MB_OK|MB_ICONSTOP "配置自启动失败（退出码 $0）。"
    Abort
  ${EndIf}

  DetailPrint "步骤 8/10 写入卸载注册表项"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayName" "FOX输入法"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayVersion" "${FOX_VERSION}"
  WriteRegStr HKLM "${UNINST_KEY}" "Publisher" "FOX"
  WriteRegStr HKLM "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${UNINST_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoRepair" 1

  ; IFEO 高优先级（自定义段承载）：Windows 功耗管理会把空闲的算法服务压到
  ; 0.5-1GHz，按键后频率爬升慢 → 选词"莫名其妙卡顿"（weasel#1250 同类问题）。
  ; CpuPriorityClass=3 (High)，让 algo/host 每次启动自动以高优先级运行，
  ; 无需进程自身提权（提权进程的 IPC 管道会拒绝普通应用）。
  DetailPrint "步骤 8.5/10 IFEO 高优先级"
  WriteRegDWORD HKLM "${IFEO_ROOT}\shurufa-algo.exe\PerfOptions" "CpuPriorityClass" 3
  WriteRegDWORD HKLM "${IFEO_ROOT}\shurufa-clipd.exe\PerfOptions" "CpuPriorityClass" 3
  WriteRegDWORD HKLM "${IFEO_ROOT}\shurufa-ui.exe\PerfOptions" "CpuPriorityClass" 3

  DetailPrint "步骤 9/10 启动宿主与 ctfmon"
  StrCpy $0 "$INSTDIR\shurufa-clipd.exe"
  StrCpy $1 "supervise"
  Call LaunchAsUser
  Exec '"$SYSDIR\ctfmon.exe"'

  DetailPrint "步骤 10/10 终态验证"
  nsExec::ExecToLog 'powershell -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "$INSTDIR\verify-install.ps1" -InstallDir "$INSTDIR"'
  Pop $0
  ${If} $0 != 0
    MessageBox MB_OK|MB_ICONSTOP "安装终态验证未通过（退出码 $0），详见 $INSTDIR\verify-install.ps1 输出。"
    Abort
  ${EndIf}
SectionEnd

Section "开始菜单快捷方式" SecStartMenu
  SetShellVarContext all
  CreateDirectory "$SMPROGRAMS\FOX"
  CreateShortCut "$SMPROGRAMS\FOX\FOX输入法.lnk" "$INSTDIR\Shurufa.exe"
SectionEnd

; 默认输入法覆盖：直接写注册表，绕开偶发挂起的 WinRT
; Set-WinDefaultInputMethodOverride（实测可在非交互会话中永久阻塞）。
Section /o "设为默认输入法" SecDefaultIme
  WriteRegStr HKCU "Control Panel\International\User Profile" "InputMethodOverride" "${DEFAULT_IME_TIP}"
SectionEnd

Section "桌面快捷方式" SecDesktop
  SetShellVarContext current
  CreateShortCut "$DESKTOP\FOX输入法.lnk" "$INSTDIR\Shurufa.exe"
SectionEnd

;──────────────────────── 卸载 ────────────────────────

Section "Uninstall"
  StrCpy $0 "Shurufa.exe"
  Call un.StopProc
  StrCpy $0 "shurufa-ui.exe"
  Call un.StopProc
  StrCpy $0 "shurufa-clipd.exe"
  Call un.StopProc
  StrCpy $0 "shurufa-algo.exe"
  Call un.StopProc
  StrCpy $0 "ctfmon.exe"
  Call un.StopProc
  StrCpy $0 "TextInputHost.exe"
  Call un.StopProc
  Call un.UnregisterTsf

  nsExec::ExecToLog 'powershell -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "$INSTDIR\register-host-startup.ps1" -Remove'
  DeleteRegValue HKCU "Control Panel\International\User Profile" "InputMethodOverride"
  DeleteRegKey HKLM "${UNINST_KEY}"
  ; 还原 IFEO（清除安装时写入的高优先级配置）
  DeleteRegValue HKLM "${IFEO_ROOT}\shurufa-algo.exe\PerfOptions" "CpuPriorityClass"
  DeleteRegKey HKLM "${IFEO_ROOT}\shurufa-algo.exe\PerfOptions"
  DeleteRegKey HKLM "${IFEO_ROOT}\shurufa-algo.exe"
  DeleteRegValue HKLM "${IFEO_ROOT}\shurufa-host.exe\PerfOptions" "CpuPriorityClass"
  DeleteRegKey HKLM "${IFEO_ROOT}\shurufa-host.exe\PerfOptions"
  DeleteRegKey HKLM "${IFEO_ROOT}\shurufa-host.exe"
  DeleteRegValue HKLM "${IFEO_ROOT}\shurufa-clipd.exe\PerfOptions" "CpuPriorityClass"
  DeleteRegKey HKLM "${IFEO_ROOT}\shurufa-clipd.exe\PerfOptions"
  DeleteRegKey HKLM "${IFEO_ROOT}\shurufa-clipd.exe"
  DeleteRegValue HKLM "${IFEO_ROOT}\shurufa-ui.exe\PerfOptions" "CpuPriorityClass"
  DeleteRegKey HKLM "${IFEO_ROOT}\shurufa-ui.exe\PerfOptions"
  DeleteRegKey HKLM "${IFEO_ROOT}\shurufa-ui.exe"

  SetShellVarContext all
  Delete "$SMPROGRAMS\FOX\FOX输入法.lnk"
  RMDir "$SMPROGRAMS\FOX"
  SetShellVarContext current
  Delete "$DESKTOP\FOX输入法.lnk"

  RMDir /r "$INSTDIR"
SectionEnd

Function un.StopProc
  StrCpy $1 0
  ${DoWhile} $1 < 3
    nsExec::ExecToLog 'taskkill /f /im $0'
    nsExec::ExecToLog 'powershell -NoProfile -NonInteractive -WindowStyle Hidden -Command "Get-CimInstance Win32_Process -Filter \"Name='''$0'''\" | ForEach-Object { Invoke-CimMethod -InputObject $$_. -MethodName Terminate | Out-Null }"'
    Sleep 1200
    IntOp $1 $1 + 1
  ${Loop}
FunctionEnd

Function un.UnregisterTsf
  IfFileExists "$INSTDIR\shurufa_tsf-${FOX_VERSION}.dll" 0 +2
    nsExec::ExecToLog '"$SYSDIR\regsvr32.exe" /s /u "$INSTDIR\shurufa_tsf-${FOX_VERSION}.dll"'
  IfFileExists "$INSTDIR\shurufa_tsf.dll" 0 +2
    nsExec::ExecToLog '"$SYSDIR\regsvr32.exe" /s /u "$INSTDIR\shurufa_tsf.dll"'
  FindFirst $3 $2 "$INSTDIR\shurufa_tsf-*.dll"
  ${DoWhile} $2 != ""
    nsExec::ExecToLog '"$SYSDIR\regsvr32.exe" /s /u "$INSTDIR\$2"'
    FindNext $3 $2
  ${Loop}
  FindClose $3
FunctionEnd
