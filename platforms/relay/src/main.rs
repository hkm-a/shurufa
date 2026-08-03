//! shurufa-relay：用于跨网段的自托管透明中继。
//!
//! 用法：shurufa-relay [监听地址]
//! 默认监听 0.0.0.0:48633；中继不终止 TLS，也不保存剪贴板内容。

fn main() {
    let addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "0.0.0.0:48633".to_string());
    let runtime = tokio::runtime::Runtime::new().expect("创建中继运行时失败");
    println!("Shurufa 中继监听：{addr}");
    if let Err(error) = runtime.block_on(sync_core::run_relay(&addr)) {
        eprintln!("中继退出：{error}");
        std::process::exit(1);
    }
}
