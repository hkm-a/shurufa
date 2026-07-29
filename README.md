# shurufa（暂名）

开源跨平台输入法套件：中文输入法 + 跨设备剪贴板同步 + 截图贴图工具。

## 三大能力

- **输入法**：基于 [librime](https://github.com/rime/librime) 引擎，Windows（TSF）与 Android 双端，内置雾凇拼音等 Rime 生态方案。
- **剪贴板同步**：本地剪贴板历史 + 局域网多设备端到端加密同步（对标微信输入法）。
- **截图贴图**：区域截图、标注、贴图 Pin、OCR 取词（对标 PixPin，桌面端）。

## 状态

项目处于架构设计阶段，尚未开始编码。设计文档见 [docs/architecture.md](docs/architecture.md)。

## 仓库结构（规划）

```
core/         Rust 跨平台核心（引擎桥接、剪贴板、同步、配置）
platforms/    Windows（TSF）与 Android 前端
screenshot/   桌面截图组件
schemas/      内置 Rime 输入方案
docs/         设计文档
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
