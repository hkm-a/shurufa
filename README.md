# Shurufa

跨平台中文输入与设备协作工具，覆盖 Windows 桌面端和 Android 输入法。

[![许可证](https://img.shields.io/badge/许可证-GPL--3.0-blue.svg)](https://www.gnu.org/licenses/gpl-3.0.html)
[![平台](https://img.shields.io/badge/平台-Windows%20%7C%20Android-2ea44f.svg)](#平台支持)
[![状态](https://img.shields.io/badge/状态-M6%20增强-2da44e.svg)](docs/m4-verification.md)

Shurufa 把输入、剪贴板和截图工具放在同一条工作流里：输入法负责稳定输入，剪贴板负责设备间同步，桌面端工具负责截图、OCR、贴图和录屏。常用功能在后台完成，不要求用户学习新的同步或文件管理流程。

## 功能概览

| 模块 | 能力 | 平台 |
| --- | --- | --- |
| 中文输入 | 基于 librime 的雾凇拼音、候选词、词频学习、模糊音 | Windows、Android |
| 剪贴板同步 | 文本、图片、文件、历史记录，局域网直连与自托管中继 | Windows、Android |
| 截图工具 | 区域/窗口截图、标注、长截图、贴图 Pin、中文 OCR | Windows |
| 录屏 | 后台录制 MP4，独立于图片剪贴板历史 | Windows |
| 词库与皮肤 | 固定版本的 rime-ice 云词库、SHA-256 校验、跨端皮肤 JSON | Windows、Android |

## 平台支持

- **Windows**：TSF 原生输入法、常驻剪贴板同步、截图/OCR/贴图/录屏、设置页和单文件安装器。
- **Android**：系统输入法服务、后台剪贴板同步、候选栏、QWERTY/符号键盘、历史面板和云词库更新。
- **同步中继**：可部署在自有服务器上，不依赖项目提供的公共服务，详见[自托管同步中继](docs/self-hosted-relay.md)。

## 快速开始

### Windows

1. 从源码构建宿主和安装器：

   ```powershell
   .\scripts\build-installer.ps1
   ```

2. 以管理员身份运行生成的 `dist\Shurufa-Setup.exe`。
3. 安装器会部署 TSF、算法服务、后台同步和默认雾凇拼音方案。

完整的升级、卸载和失败恢复说明见[Windows 安装指南](docs/windows-installation.md)。

### Android

Android 构建需要 Android SDK、JDK 17 和 Rust Android target：

```powershell
cargo build --release --target x86_64-linux-android -p shurufa-rimejni
gradle.bat -p platforms/android :app:assembleDebug --console=plain
```

生成的 APK 位于 `platforms/android/app/build/outputs/apk/debug/`。首次使用时，将 Shurufa 设为系统默认输入法即可。

### 词库与同步

- [云词库说明](docs/cloud-dictionary.md)：默认 rime-ice 版本、镜像回退和本地校验。
- [自托管同步中继](docs/self-hosted-relay.md)：服务器部署、端口和配对流程。
- [架构说明](docs/architecture.md)：核心模块、协议和平台集成点。

## 从源码构建

项目使用 Rust workspace、Kotlin Android 工程和原生 Windows 工程。Windows 的 Rime 依赖不入库，首次构建前准备对应版本的 librime：

```powershell
New-Item -ItemType Directory -Force third_party\librime | Out-Null
Invoke-WebRequest `
  -Uri https://github.com/rime/librime/releases/download/1.17.0/rime-33e7814-Windows-msvc-x64.7z `
  -OutFile third_party\librime\rime.7z
7z x -y third_party\librime\rime.7z -othird_party\librime
```

常用本地验证：

```powershell
cargo test --workspace
gradle.bat -p platforms/android lintDebug testDebugUnitTest --console=plain
git diff --check
```

## 仓库结构

```text
core/          Rust 跨平台核心、剪贴板、输入桥接和同步协议
platforms/     Windows TSF/桌面端、Android 输入法、中继和设置页
schemas/       Rime 输入方案、词库清单和跨端皮肤
docs/          架构、安装、云词库、中继和里程碑验证文档
scripts/       构建、安装、注册和端到端验收脚本
installer/     Windows 单文件安装器脚本
```

## 当前状态

当前实现已完成 M6 增强目标。最近一次本地验收覆盖：Android lint 与单元测试、Windows 宿主回归、同步核心协议测试、JNI 构建，以及 Android 模拟器后台双向图片同步。详细评分和证据保存在[验证报告](.claude/verification-report.md)中。

## 许可证

本项目采用 GPL-3.0。第三方 librime、rime-ice 和 OCR 组件继续遵循各自许可证；分发时请同时阅读对应上游项目的许可条款。
