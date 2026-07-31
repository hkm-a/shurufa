# 开发环境约定

## 终端/脚本运行环境（重要）

**本机已安装 PowerShell 7 与 WSL，一律不要使用 Windows PowerShell 5.1（`powershell.exe`）执行项目脚本或命令。**

- PowerShell 7：`pwsh`（`C:\Users\hkm\AppData\Local\Microsoft\WindowsApps\pwsh`，版本 7.6.4）
- WSL：`wsl`（`C:\WINDOWS\system32\wsl`）
- Windows PowerShell 5.1（`powershell.exe` / `C:\WINDOWS\System32\WindowsPowerShell\v1.0\powershell.exe`）**禁用**，原因：

### 为什么不用 PS 5.1

1. **原生命令 stderr 被 `$ErrorActionPreference='Stop'` 当作终止错误**：
   PS 5.1 中，原生命令（adb、cargo、gradlew 等）向 stderr 写内容时，即使命令成功（如 cargo 的 `Finished` 输出），也会在 `$ErrorActionPreference='Stop'` 下抛出
   `NativeCommandError`，导致脚本一启动就挂。PS 7 不再把 stderr 当作终止错误。
2. 脚本内曾因此需要 `2>&1 | ForEach-Object { "$_" }` 之类的规避写法（见
   `scripts/test-android-attachments-emulator.ps1` 的 `Invoke-Checked`），PS 7 下无此问题。
3. PS 5.1 对后台重定向、长命令引号处理等也有坑（历史教训：日志文件未被写入、命令被截断）。

### 执行约定

- 所有 PowerShell 命令/脚本一律用 `pwsh` 执行：
  ```powershell
  pwsh -NoProfile -ExecutionPolicy Bypass -File .\scripts\xxx.ps1
  ```
- 后台长时间任务（模拟器测试等）使用 `pwsh` + 重定向到日志文件的方式运行，勿用 PS 5.1。
- Android SDK / adb 路径：`$env:LOCALAPPDATA\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\Android\Sdk`
- JDK 17：`C:\Program Files\Microsoft\jdk-17.0.20.8-hotspot`
- Gradle 发行版：SDK 目录下 `gradle-dist`

### 模拟器附件测试脚本

`scripts/test-android-attachments-emulator.ps1` 已适配 PS 5.1/PS 7 双环境（`Invoke-Checked` 做了兼容），
但日常调试请用 PS 7。后台跑全量验收的推荐方式：

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\.claude\run-emulator-tests.ps1 -KeepEmulator
```

（`.claude\run-emulator-tests.ps1` 是包装脚本，把主脚本输出重定向到 `.claude\emulator-tests.log`。）
