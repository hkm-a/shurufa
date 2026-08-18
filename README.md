# Shurufa

跨平台中文输入与设备协作工具，覆盖 Windows 桌面端和 Android 输入法。

[![许可证](https://img.shields.io/badge/许可证-GPL--3.0-blue.svg)](LICENSE)
[![平台](https://img.shields.io/badge/平台-Windows%20%7C%20Android-2ea44f.svg)](#平台支持)
[![CI](https://github.com/hkm-a/shurufa/actions/workflows/ci.yml/badge.svg)](https://github.com/hkm-a/shurufa/actions/workflows/ci.yml)
[![状态](https://img.shields.io/badge/状态-v0.8.0-2da44e.svg)](CHANGELOG.md)

Shurufa 把中文输入和设备剪贴板放在同一条工作流里：输入法负责稳定输入，剪贴板负责设备间同步。截图、标注与录屏由 PixPin 等专业工具负责，Shurufa 不再重复提供这类桌面能力。

## 功能概览

| 模块 | 能力 | 平台 |
| --- | --- | --- |
| 中文输入 | 基于 librime 的雾凇拼音、候选词、词频学习、模糊音 | Windows、Android |
| 剪贴板同步 | 文本、图片、文件、历史记录，局域网直连与自托管中继 | Windows、Android |
| 词库与皮肤 | 固定版本的 rime-ice 云词库、SHA-256 校验、跨端皮肤 JSON | Windows、Android |
| 控制中心 | Tauri 桌面端：历史、设备、皮肤、词库、中继设置 | Windows |

## 平台支持

- **Windows**：TSF 原生输入法、常驻剪贴板同步、设置页和带版本号的单文件安装器（`Shurufa-Setup-<版本>.exe`，附 SHA-256）。
- **Android**：系统输入法服务、后台剪贴板同步、候选栏、QWERTY/符号键盘、历史面板和云词库更新。
- **同步中继**：可部署在自有服务器上，不依赖项目提供的公共服务，详见 [自托管同步中继](docs/自托管中继.md)。

## 快速开始

### 用户

- **Windows**：从 [Release](../../releases) 下载 `Shurufa-Setup-<版本>.exe`，右键"以管理员身份运行"。详见 [Windows 安装指南](docs/Windows安装指南.md)。
- **Android**：从 [Release](../../releases) 下载 APK 安装，然后在系统设置中启用 Shurufa。详见 [Android 安装与使用](docs/安卓安装与使用.md)。

### 开发者

Windows 一键构建并打包：

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-installer.ps1
```

产物位于 `dist\Shurufa-Setup-<版本>.exe`，同目录附 `.sha256` 校验和。构建机需要 NSIS；已 Release 构建过可加 `-SkipBuild`；正式发布请配 `SHURUFA_SIGN_PFX`/`SHURUFA_SIGN_PASSWORD` 后加 `-Sign`。

Android 构建（需 JDK 17、Android SDK 与 Rust Android target）：

```powershell
.\scripts\build-android.cmd
```

常用本地验证：

```powershell
cargo test --workspace --locked
.\scripts\set-version.ps1 -Check
git diff --check
```

## 仓库结构

```text
core/          Rust 跨平台核心、剪贴板、输入桥接和同步协议
platforms/     Windows TSF/桌面端、Android 输入法、中继和设置页
schemas/       Rime 输入方案、词库清单和跨端皮肤（双端单一事实源）
docs/          中英文对照的产品/架构/验收/发布文档
installer/     NSIS 安装器脚本与共享部署模块 Deploy-Shurufa.ps1
scripts/       构建、版本 bump、开发部署、端到端验收脚本
version.json   版本号单一事实源（由 set-version.ps1 同步四处派生点）
```

## 文档导航

- 产品：[架构说明](docs/架构说明.md) · [迭代方向](docs/迭代方向.md) · [开发计划](docs/开发计划.md)
- 用户：[Windows 安装指南](docs/Windows安装指南.md) · [安卓安装与使用](docs/安卓安装与使用.md) · [云词库](docs/云词库.md) · [自托管中继](docs/自托管中继.md)
- 开发者：[开发环境](docs/开发环境.md) · [发布流程](docs/发布流程.md) · [版本管理](docs/版本管理.md) · [文档管理](docs/文档管理.md) · [CHANGELOG](CHANGELOG.md)
- 验收：[M1](docs/M1-验收报告.md) · [M2](docs/M2-验收报告.md) · [M3](docs/M3-验收报告.md) · [M4 Windows](docs/M4-验收报告.md) · [M4 Android](docs/M4-安卓验收报告.md) · [M7](docs/M7-验收报告.md) · [M8](docs/M8-验收报告.md) · [安卓附件测试](docs/安卓附件测试.md)

## 当前状态

当前版本 **v0.9.0（versionCode 31，2026-08-18）**，M8「跨设备与个性化深化」已完成：
跨设备同步状态可视化与失败重试、设备管理（重命名/移除/最近在线）、剪贴板批量整理、
应用/网站直达候选、皮肤包导入导出；承接 M7 的多行候选面板、候选条右键菜单、
悬浮球不透明度、上下文调频、多时机表情推荐（M8 验收见
[M8 验收报告](docs/M8-验收报告.md)，M7 见 [M7 验收报告](docs/M7-验收报告.md)），
路线图见 [开发计划](docs/开发计划.md)：

- **输入**：librime 雾凇拼音 + 小鹤双拼、MRU 最近使用提频、音节分词视图、
  Shift/CapsLock/Ctrl+. 切换、全局中/英语义、V 模式 / 以词定字 / 辅码检字 /
  错音提示 / 冷词丢弃 / Emoji 候选 / Unicode 输入 / 符号配对 / 中英混输自动空格
  等 rime-ice 同款能力，五笔/仓颉入口预留。
- **桌面**：FOX 悬浮球（中/En 指示、剪贴板历史、语音转写、AI 帮写/润色/翻译、
  划词入口）、皮肤系统（5 套预设 + JSON 编辑器 + 热生效）、候选窗 DComp/D2D/GDI
  瀑布渲染、模式切换 toast、Tauri 自研安装器。
- **跨设备**：剪贴板文本/图片/文件双向同步（协议 v3：分块校验 + 回执）、文件接收
  审批通知、剪贴板收藏、自托管中继、云词库多代回滚。
- **工程**：全工作区 cargo clippy（-D warnings）与 cargo fmt --check 零告警、
  约 199 项测试全绿、版本单一事实源 0.9.0/31、CI 双平台（Windows/Android）流水线。

详细验收证据与评分见 [验证报告](.claude/verification-report.md)，演进过程见 [CHANGELOG](CHANGELOG.md)。

## 许可证

本项目采用 GPL-3.0，详见 [LICENSE](LICENSE)。第三方 librime 和 rime-ice 组件继续遵循各自许可证；分发时请同时阅读对应上游项目的许可条款。
