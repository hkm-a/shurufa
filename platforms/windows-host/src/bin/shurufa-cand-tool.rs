//! 便携候选窗验收小工具（无需 Python / 无需安装环境）。
//!
//! 单个 exe 即可完成候选窗 hosted 模式的快速验证：
//! - `--selftest`：跑 `cand_host::selftest()`（起服务 → 推 Show → 断言窗口）；
//! - `--demo [秒]`：启动 cand_host 并显示一个示例候选窗，持续 N 秒后退出；
//! - `--info`：打印显示器数量 / 系统 DPI 等环境信息。
//!
//! 用法：
//!   shurufa-cand-tool.exe --selftest
//!   shurufa-cand-tool.exe --demo 5
//!   shurufa-cand-tool.exe --info

use std::time::{Duration, Instant};

use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetSystemMetrics, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    SM_CMONITORS, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
};

use ime_ipc::{encode_cand_event, CandEvent, Candidate, Context};
use windows_ipc::pipe::PipeClient;

/// 泵消息直到 deadline；同时返回是否仍有候选窗存活（简单探活）。
fn pump_until(deadline: Instant) {
    let mut msg = MSG::default();
    while Instant::now() < deadline {
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                let _ = DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn demo(seconds: u64) -> i32 {
    if !shurufa_host::cand_host::start() {
        eprintln!("[cand-tool] cand_host 启动失败");
        return 1;
    }
    // 等 accept 线程就绪
    std::thread::sleep(Duration::from_millis(200));
    let client = match PipeClient::connect_named(windows_ipc::pipe::CAND_PIPE_NAME) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[cand-tool] 连接候选窗管道失败：{e}");
            return 1;
        }
    };
    let event = CandEvent::Show {
        client_id: 4242,
        context: Context {
            preedit: "nihao".into(),
            candidates: vec![
                Candidate {
                    text: "你好".into(),
                    comment: String::new(),
                },
                Candidate {
                    text: "拟好".into(),
                    comment: String::new(),
                },
                Candidate {
                    text: "泥嚎".into(),
                    comment: String::new(),
                },
            ],
            highlighted: 0,
            page_size: 9,
            ..Context::default()
        },
        caret_rect: (300, 300, 8, 16),
        dpi: 96,
        multi_line: false,
        position: "follow".to_owned(),
    };
    let frame = match encode_cand_event(&event) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[cand-tool] 编码事件失败：{e}");
            return 1;
        }
    };
    if let Err(e) = client.write_frame(&frame) {
        eprintln!("[cand-tool] 推送 Show 失败：{e}");
        return 1;
    }
    println!("[cand-tool] 示例候选窗已显示，持续 {seconds}s…");
    pump_until(Instant::now() + Duration::from_secs(seconds));
    println!("[cand-tool] 演示结束");
    0
}

fn info() -> i32 {
    unsafe {
        let monitors = GetSystemMetrics(SM_CMONITORS);
        let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        let dpi = GetDpiForSystem();
        println!("显示器数量: {monitors}");
        println!("虚拟屏尺寸: {vw}x{vh}");
        println!("系统 DPI: {dpi}（{}% 缩放）", dpi * 100 / 96);
    }
    0
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("--selftest") => shurufa_host::cand_host::selftest(),
        Some("--demo") => {
            let seconds = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5);
            demo(seconds)
        }
        Some("--info") => info(),
        _ => {
            println!(
                "便携候选窗验收工具\n\
                 用法:\n\
                 \x20 shurufa-cand-tool.exe --selftest\n\
                 \x20 shurufa-cand-tool.exe --demo [秒]\n\
                 \x20 shurufa-cand-tool.exe --info"
            );
            0
        }
    };
    std::process::exit(code);
}
