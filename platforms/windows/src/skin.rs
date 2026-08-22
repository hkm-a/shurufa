//! Windows 皮肤入口：直接 re-export `windows_skin`。
//!
//! 阶段 4 拆分后，纯模型/解析在 `core/skin`，Windows 专属装载/DWM 助手在
//! `platforms/windows-skin`；本文件只保留 `crate::skin::*` 的旧路径兼容，
//! 避免候选窗/面板代码大范围改名。
//!
//! 注意：`SkinExt` 也通过 glob re-export 带出，因此 `Skin::current()` 等
//! trait 方法在 `use crate::skin::*` 的模块里可直接调用。

pub use windows_skin::*;
