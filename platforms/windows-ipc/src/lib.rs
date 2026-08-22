//! Windows 命名管道传输层。
//!
//! 阶段 4 拆分：`core/ime-ipc` 只保留跨平台协议与 DTO；Windows 专属的
//! 命名管道 `pipe` 与算法服务接入 `server` 移到本 crate，让 `core/ime-ipc`
//! 能在非 Windows target 上通过 `cargo check`。

pub mod pipe;
pub mod server;
