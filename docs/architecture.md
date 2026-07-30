# 架构设计文档

## 1. 项目定位

开源跨平台输入法套件，包含三大能力：

1. **输入法核心**：拼音等中文输入方案，桌面（Windows 优先）与 Android 双端。
2. **剪贴板同步**：多设备剪贴板历史管理与跨设备同步（对标微信输入法）。
3. **截图工具**：截图、贴图（Pin）、标注、OCR 取词（对标 PixPin），桌面端能力。

## 2. 总体原则：标准化 + 生态复用

不自研输入法引擎。中文输入引擎是十年级工程量（词库、语言模型、模糊音、造词、云候选），社区已有成熟方案：

| 能力 | 复用方案 | 许可证 | 说明 |
|---|---|---|---|
| 输入引擎 | [librime](https://github.com/rime/librime)（C++） | BSD-3 | Rime 核心，方案驱动，全平台可移植 |
| Windows 前端参考 | [weasel（小狼毫）](https://github.com/rime/weasel) | GPL-3 | TSF 集成的完整参考实现 |
| Android 前端参考 | [trime（同文）](https://github.com/osfans/trime) / [fcitx5-android](https://github.com/fcitx5-android/fcitx5-android) | GPL-3 | InputMethodService + JNI 桥接 librime 的参考 |
| 词库/方案 | rime 生态方案（朙月拼音、雾凇拼音等） | 各自许可 | 直接以子模块/下载方式分发 |
| 截图参考 | [Flameshot](https://github.com/flameshot-org/flameshot)（C++/Qt）、[ShareX](https://github.com/ShareX/ShareX) | GPL-3 | 标注交互与贴图设计参考 |
| 剪贴板管理参考 | [EcoPaste](https://github.com/EcoPasteHub/EcoPaste)（Tauri/Rust） | Apache-2.0 | 剪贴板历史 UI 与格式处理参考 |
| 设备互联参考 | [KDE Connect](https://invent.kde.org/network/kdeconnect-kde) / [LocalSend](https://github.com/localsend/localsend) | GPL/MIT | 局域网发现 + 配对 + 加密通道的成熟模式 |
| OCR | [PaddleOCR](https://github.com/PaddlePaddle/PaddleOCR) 或系统 OCR（Windows.Media.Ocr / ML Kit） | Apache-2.0 | 截图取词 |

> 许可证注意：weasel/trime 为 GPL-3，若直接复用其代码，对应组件需以 GPL 兼容方式开源；仅作参考重写则不受约束。librime 为 BSD，可自由链接。

## 3. 顶层架构

```
┌──────────────────────────────────────────────────────┐
│                     各平台前端（Shell）               │
│  Windows: TSF 文本服务 + 候选窗(WinUI/Qt)             │
│  Android: InputMethodService + Compose 键盘 UI        │
├──────────────────────────────────────────────────────┤
│                核心层 core/（Rust，跨平台）           │
│  ime-bridge     librime 封装（FFI），会话/候选管理     │
│  clipboard      剪贴板历史存储（SQLite）、格式归一化   │
│  sync           设备发现、配对、端到端加密同步协议     │
│  config         方案/皮肤/设置，兼容 Rime YAML        │
├──────────────────────────────────────────────────────┤
│              桌面独立组件（与输入法进程解耦）          │
│  screenshot/    截图、贴图、标注、OCR（桌面端）        │
└──────────────────────────────────────────────────────┘
```

核心层用 **Rust**：一份代码同时服务桌面（cdylib/静态库）与 Android（JNI，经 uniffi 或手写绑定），生态里 SQLite（rusqlite）、网络（quinn/tokio）、加密（ring/rustls）都是成熟库，符合复用原则。librime 本体是 C++，通过 `librime-sys` 风格的 FFI 绑定接入。

## 4. 各模块设计

### 4.1 输入法核心（core/ime-bridge + 平台前端）

- **引擎**：librime 以预编译库 + FFI 接入；输入方案直接使用 Rime 生态（默认内置雾凇拼音）。
- **Windows 前端**：TSF（Text Services Framework）实现 `ITfTextInputProcessor`，键事件转发给 librime 会话，候选窗为独立顶层窗口。整体流程参考 weasel 的 `WeaselTSF` 结构，但候选窗 UI 自绘以支持皮肤。
- **Android 前端**：Kotlin `InputMethodService`，键盘布局与候选栏用 Jetpack Compose；通过 JNI 调用同一个 Rust 核心层。参考 fcitx5-android 的服务生命周期处理（横竖屏、密码框禁用联想等）。
- **进程模型（桌面）**：算法服务常驻进程 + 每应用内的轻量 TSF 客户端，命名管道通信（weasel 同款模型），避免引擎在每个宿主进程里重复加载词库。

### 4.2 剪贴板历史与同步（core/clipboard + core/sync）

- **本地历史**：监听系统剪贴板（Windows `AddClipboardFormatListener`；Android 受系统限制，仅输入法处于活跃状态时可读，须在 UI 中明示），归一化为 文本/图片/文件引用 三类，存 SQLite，支持置顶、搜索、过期清理。
- **同步拓扑**：优先**局域网直连**（mDNS 发现 + 加密通道），无公网服务器依赖；后续可选自托管中继（类似 Syncthing relay）解决跨网段场景。
- **传输实现（M4 落地）**：MVP 采用 **TCP + rustls(TLS 1.3)** 而非架构初稿的 QUIC。剪贴板同步只需单条有序流，QUIC 的多路复用与连接漫游优势用不上，而 tokio-rustls 的集成面比 QUIC 栈小一个数量级。传输层已隔离在 `core/sync` 的 service 模块内，后续如需 QUIC/中继可替换而不动上层协议。
- **配对与加密**（不可省略）：每台设备生成自签证书（rcgen），SHA-256 指纹即设备身份。首次配对走 **KDE Connect 式六位确认码**——两端各自从双方指纹推导出相同数字，用户人眼比对后放行（无需摄像头扫码，桌面手机通用），确认后互相钉扎指纹。此后所有连接 TLS 1.3 双向证书认证，握手后校验对端指纹必须在已配对表中。剪贴板内容含密码与验证码，明文传输不可接受。
- **策略**：默认只同步文本、单条大小限额、敏感应用（密码管理器）来源默认不入历史。

### 4.3 截图与贴图（screenshot/，桌面）

- 独立可执行组件，与输入法共用托盘与设置中心，但崩溃互不影响。
- 功能分期：区域/窗口截图 → 标注（矩形/箭头/马赛克/文字）→ 贴图 Pin（置顶无边框窗口，支持缩放/透明度）→ OCR 取词 → 长截图。
- 技术选型：Windows 上用 `Windows.Graphics.Capture` 抓屏；UI 与标注层若核心层已选 Rust，可用 Tauri/egui，或直接以 Qt 复用 Flameshot 的交互设计。OCR 首选系统自带 `Windows.Media.Ocr`（零依赖），PaddleOCR 作为高精度可选后端。
- Android 端截图非输入法职责且权限受限，标注能力以「处理系统截图/分享进来的图片」形式提供，不做后台抓屏。

### 4.4 配置与皮肤

- 配置格式沿用 Rime YAML（方案层）+ 应用自身 TOML（外观、同步、快捷键），避免发明新格式。
- 皮肤定义一套跨端 JSON/TOML 描述（配色、圆角、字体、按键布局），桌面候选窗与 Android 键盘共用语义。

## 5. 仓库结构（Monorepo）

```
shurufa/
├── core/                  # Rust workspace：ime-bridge / clipboard / sync / config
├── platforms/
│   ├── windows/           # TSF 前端 + 服务进程 + 安装器(WiX)
│   └── android/           # Gradle 工程，InputMethodService + Compose
├── screenshot/            # 桌面截图组件
├── schemas/               # 内置 Rime 方案（子模块或构建期拉取）
├── docs/                  # 架构、协议、贡献指南
└── .claude/               # 工作流文档
```

## 6. 里程碑路线图

| 阶段 | 目标 | 关键交付 |
|---|---|---|
| M0 骨架 | 构建体系跑通 | Rust workspace + librime FFI 冒烟测试（喂键得候选） |
| M1 Windows MVP-可用版 | 能日常打字 | TSF 注册、候选窗、雾凇拼音、设置页、安装器 |
| M2 剪贴板本地 | 单机历史 | 监听/存储/搜索/粘贴回填，历史面板 |
| M3 Android | 手机可用 | 键盘 UI、JNI 桥、与桌面同方案词库 |
| M4 同步 | 跨设备 | mDNS 发现、扫码配对、加密通道、文本同步 |
| M5 截图 | 桌面截图 | 区域截图 + 标注 + 贴图 |
| M6 增强 | 体验补全 | OCR、皮肤系统、云词库、自托管中继 |

## 7. 关键风险

1. **TSF 兼容性**是 Windows 输入法最大的坑（游戏全屏、UWP、管理员进程），必须尽早在真实应用矩阵（Office/浏览器/游戏/终端）里冒烟，weasel 的 issue 列表是现成的兼容性清单。
2. **Android 剪贴板限制**：Android 10+ 后台应用不可读剪贴板，例外是被设为默认输入法的应用；同步功能依赖"默认输入法"身份，产品引导要按此设计。
3. **GPL 传染**：参考 weasel/trime 代码需隔离到对应 GPL 组件，或者只学设计不抄实现；建议主仓库整体 GPL-3 以最大化可复用范围，这是输入法开源社区的主流选择。
4. **librime 在 Android 的构建**（NDK 交叉编译 boost/glog 依赖）有现成先例（trime、fcitx5-android 的构建脚本），直接复用其 CMake 工具链配置。
5. **进程内引擎的用户词库锁冲突（已实测确认，2026-07-29）**：每个宿主进程各自加载 librime 时，Rime 用户词库（leveldb）持排他锁，仅首个进程能打开 `*.userdb/LOCK`，其余进程报 `IO error: LOCK 被占用` 并降级运行——打字不受影响，但用户造词/调频只在抢到锁的进程生效。根治方案是 weasel 同款的独立算法服务进程 + 轻量 TSF 客户端（命名管道 IPC），列入 M6 前必做的架构演进；届时候选窗也随服务进程迁出宿主，一并解决沙箱宿主内 UI 受限问题。
