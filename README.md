# shurufa（暂名）

开源跨平台输入法套件：中文输入法 + 跨设备剪贴板同步 + 截图贴图工具。

## 三大能力

- **输入法**：基于 [librime](https://github.com/rime/librime) 引擎，Windows（TSF）与 Android 双端默认内置雾凇拼音核心词典；支持本地首选词学习、长词/整句优先，以及 n/l、前后鼻音模糊音。
- **剪贴板同步**：本地剪贴板历史 + 局域网多设备端到端加密同步（对标微信输入法）。
- **截图贴图**：区域截图、标注、贴图 Pin、OCR 取词（对标 PixPin，桌面端）。

## 状态

已实现至 **M5 截图**（桌面区域截图/贴图/OCR/录屏、Android 键盘与同步），
文档永续推进中，详见 [docs/architecture.md](docs/architecture.md) 与各里程碑验证文档。

已落地能力：
- **输入法**：Windows TSF 与 Android 双端基于 librime 引擎，内置雾凇拼音；
  Android 端含 QWERTY/符号键盘、候选栏、贴纸建议、剪贴板历史面板。
- **剪贴板同步**：本地 SQLite 历史 + 局域网 TLS 1.3 双向证书端到端同步
  （mDNS 桌面端发现 / 手机 IP 直连配对，六位码确认，指纹钉扎）。
- **截图贴图**（桌面 `windows-host`）：区域/窗口截图、标注、贴图 Pin、
  OCR、屏幕录制。

> 注：仓库从「架构设计」演进为可运行实现，此 README 由各里程碑同步维护。

## 仓库结构

```
core/         Rust 跨平台核心（引擎桥接、剪贴板、同步）
platforms/    Windows（TSF + 桌面常驻 windows-host）与 Android（Kotlin + rimejni JNI）
schemas/      内置 Rime 输入方案
docs/         设计文档与里程碑验证
scripts/      构建/注册/测试脚本
```

## 构建

依赖预编译 librime（不入库），首次构建前执行：

```bash
mkdir -p third_party/librime && cd third_party/librime && curl -sL -o rime.7z https://github.com/rime/librime/releases/download/1.17.0/rime-33e7814-Windows-msvc-x64.7z && 7z x -y rime.7z
```

然后运行冒烟测试（首次会编译词典，约数十秒）：

```bash
cargo test -p ime-bridge --test smoke -- --nocapture
```

## 许可证

GPL-3.0（规划）——与 Rime 桌面/移动前端生态保持一致，便于互相复用代码。
