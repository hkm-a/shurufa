//! 跨平台输入法策略层（stage 4 第 1 项）。
//!
//! 目标：Windows `serve_connection` 与 Android `nativeProcessKey` 退化为
//! 「解包 → 调 ime_policy → 打包」，把双端行为分叉收口到零平台依赖的
//! `core/ime-policy`。
//!
//! 当前切片：
//! - `global_ascii`：全局中/英状态机；
//! - `composition`：超长组合防护；
//! - `stats`：打字统计埋点薄封装；
//! - `mru`：最近使用候选提频（从 windows-algo 移入）；
//! - `jianpin`：简拼索引查询（从 windows-algo 移入）。

pub mod composition;
pub mod global_ascii;
pub mod jianpin;
pub mod mru;
pub mod stats;

pub use composition::{is_overlong_composition, MAX_COMPOSITION_LEN};
pub use global_ascii::{GlobalAscii, GLOBAL_ASCII};
pub use jianpin::JianpinIndex;
pub use mru::MruStore;
pub use stats::{note_commit, note_key};
