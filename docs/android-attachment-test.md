# Android 附件投递测试

测试器是独立 APK，包名为 `com.shurufa.attachmenttest`。它与输入法不共享进程或存储，因此能发现真实聊天应用同类的 URI 授权问题。

## 后台双向图片剪贴板验收

在仓库根目录运行：

```powershell
& .\scripts\test-background-image-sync-emulator.ps1
```

该脚本自动构建隔离 Windows host 和 API 35 Android APK，生成两端临时身份与配对表，经真实 TCP/TLS 链路验证：

1. 输入法窗口关闭时，Windows 41×29 位图同步为 Android `image/png` 剪贴板。
2. 输入法窗口关闭时，Android 37×23 FileProvider 图片同步为 Windows 位图剪贴板。
3. 测试结束后关闭 AVD、移除 ADB 转发并恢复日常 Windows host。

证据保存在 `.claude/background-sync-artifacts/`。测试身份、端口、数据库和日志均与日常环境隔离。

## 全自动模拟器验收

无需连接手机，直接在仓库根目录运行（本机请用 PowerShell 7，勿用 PS 5.1）：

```powershell
pwsh -ExecutionPolicy Bypass -File scripts\test-android-attachments-emulator.ps1
```

脚本会自动补齐 Android Emulator、API 35 x86_64 镜像、Rust x86_64 Android 目标和固定版本的 fcitx5-android 预编译依赖，随后创建专用 AVD、构建并安装两个 APK。测试数据由仅存在于 Debug APK 的接收器注入，真实操作仍经过输入法历史、目标 MIME 能力检测、`commitContent`、剪贴板兜底和测试器跨进程读取。

自动覆盖：

1. 富内容输入框接收图片（commitContent 声明驱动）。
2. 富内容输入框接收文件。
3. 仅声明 `image/jpeg` 的输入框接收 JPEG 图片，验证 MIME 协商和转码。
4. 未声明附件能力（同抖音/B 站评论框）的输入框：图片/文件条目点击后回退为**复制到系统剪贴板**（标签带 `Shurufa 同步·` 前缀防回环，URI 落在 MediaStore），由测试器跨进程读取验证；目标进程不收到附件。
5. 普通文本输入框不接收附件（负对照）。

> 注：**不使用 ACTION_SEND 分享兜底**。实机验证（抖音）表明分享语义是“分享到应用”而非“插入输入框”，会误触打开发布/分享界面；对不声明 MIME 的输入框一律复制到剪贴板由用户长按粘贴。

截图和日志保存在 `.claude/android-emulator-artifacts/`。默认在验收后关闭本次启动的模拟器；使用 `-KeepEmulator` 可保留，使用 `-Visible` 可显示模拟器窗口。后台运行全量验收可用 `pwsh -File .claude\run-emulator-tests.ps1 -KeepEmulator`（输出重定向到 `.claude\emulator-tests.log`）。

## 一键安装

连接并授权一台启用 USB 调试的 Android 手机，在仓库根目录运行：

```powershell
powershell -ExecutionPolicy Bypass -File scripts\test-android-attachments.ps1
```

脚本会复用本机已有 Android SDK 与 Gradle，构建并安装输入法 APK 和测试器 APK，然后自动打开测试器。

## 验证顺序

1. 在电脑复制一张图片，等待手机输入法出现同步提示。
2. 点击测试器的“标准富内容输入框”，再点击输入法中的图片条目。
3. 成功时结果应显示来源为 `commitContent`、正确 MIME、非零字节数和图片预览。
4. 点击“仅接收 JPEG 的富内容输入框”，应显示 `image/jpeg`，验证输入法已转码。
5. 点击“未声明附件能力的输入框”：图片/文件历史条目可点击，回退为复制到系统剪贴板，长按输入框粘贴即可发送（不会触发“分享到应用”）。
6. “普通文本输入框”是负对照，拒绝附件属于预期行为。
7. 用一个小文件重复上述步骤，确认名称、MIME 和字节数正确。

## 日志

```powershell
powershell -ExecutionPolicy Bypass -File scripts\test-android-attachments.ps1 -Logs
```

测试器只有在目标进程实际打开 URI 后才记录成功。`performContextMenuAction` 或 `commitContent` 单纯返回 `true` 不作为最终成功依据。
