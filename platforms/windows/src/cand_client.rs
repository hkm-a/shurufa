//! 候选窗 hosted 模式的 TSF 侧客户端（S3 双路径灰度）。
//!
//! TSF DLL 作为 `\\.\pipe\shurufa-cand` 的客户端：
//! - 上行：把 `CandEvent::Show/Hide` 全量推给 `shurufa-ui` 的 `cand_host`；
//! - 下行：读 `CandCommand`（Select/PageNext/PagePrev），用 SendInput 合成
//!   虚拟键，重走 TSF 正常按键路径（数字选词/翻页拦截全部生效）。
//!
//! S5 起 hosted 为默认路径；连接失败或管道断开时调用方回退内置绘制。

use std::io;
use std::ops::Deref;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use ime_ipc::{decode_cand_command, encode_cand_event, CandCommand, CandEvent, Context};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    MAPVK_VK_TO_VSC, VIRTUAL_KEY,
};
use windows_ipc::pipe::{PipeClient, CAND_PIPE_NAME};

/// PipeClient 只有 Send 没有 Sync；读写同一连接句柄在消息模式管道下安全
/// （WriteFile/ReadFile 原子一条消息），包一层显式 Sync 供主线程写 + 读线程读。
struct SyncPipeClient(PipeClient);

unsafe impl Sync for SyncPipeClient {}

impl Deref for SyncPipeClient {
    type Target = PipeClient;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// 单个 TSF 宿主进程的 hosted 候选客户端。
pub struct CandClient {
    pipe: Arc<SyncPipeClient>,
    _reader: JoinHandle<()>,
}

impl CandClient {
    /// 连接候选窗事件管道并启动命令读取线程。
    pub fn connect() -> Result<Self, String> {
        let pipe = PipeClient::connect_named(CAND_PIPE_NAME).map_err(|e| e.to_string())?;
        let shared = Arc::new(SyncPipeClient(pipe));
        let reader_shared = shared.clone();
        let reader = std::thread::spawn(move || read_commands(reader_shared));
        Ok(CandClient {
            pipe: shared,
            _reader: reader,
        })
    }

    /// 推送全量候选帧（ui 侧零会话状态，每帧都全量）。
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &self,
        client_id: u32,
        ctx: &Context,
        caret_rect: (i32, i32, i32, i32),
        dpi: u32,
        multi_line: bool,
        position: &str,
        inline_preedit: bool,
    ) -> Result<(), String> {
        let event = CandEvent::Show {
            client_id,
            context: ctx.clone(),
            caret_rect,
            dpi,
            multi_line,
            position: position.to_owned(),
            inline_preedit,
        };
        let frame = encode_cand_event(&event)?;
        self.pipe.write_frame(&frame).map_err(|e| e.to_string())
    }

    /// 通知 ui 隐藏当前 client 的候选窗。
    pub fn hide(&self, client_id: u32) -> Result<(), String> {
        let frame = encode_cand_event(&CandEvent::Hide { client_id })?;
        self.pipe.write_frame(&frame).map_err(|e| e.to_string())
    }
}

/// 命令读取线程：ui 点击/滚轮回发命令后，在本进程合成虚拟键。
/// 失败静默：管道断开即退出，调用方下次 show 会重连/回退。
fn read_commands(pipe: Arc<SyncPipeClient>) {
    // 注意：不能在这里用阻塞 ReadFile 长占管道读端；同一客户端句柄上
    // 并发阻塞 Read + Write 会在少量帧后互相等待（实测 hosted 模式敲到
    // 第 3 个字母就卡死）。改用 PeekNamedPipe 轮询，空闲时不占 ReadFile。
    loop {
        match pipe.read_frame_timeout(Duration::from_millis(200)) {
            Ok(frame) => {
                let Ok(cmd) = decode_cand_command(&frame) else {
                    continue;
                };
                match cmd {
                    CandCommand::Select { index, .. } => unsafe {
                        // 与候选窗数字选词一致：1..9 选前 9 项，0 选第 10 项。
                        let vk = if index < 9 {
                            0x31 + index as u16 // VK_1..VK_9
                        } else {
                            0x30 // VK_0
                        };
                        send_virtual_key(vk as u8);
                    },
                    CandCommand::PageNext { .. } => unsafe {
                        send_virtual_key(0x22); // VK_NEXT = PageDown
                    },
                    CandCommand::PagePrev { .. } => unsafe {
                        send_virtual_key(0x21); // VK_PRIOR = PageUp
                    },
                    CandCommand::MenuAction { index, action, .. } => {
                        crate::candidate_window::dispatch_menu_action(&action, index);
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::TimedOut => continue,
            Err(_) => break,
        }
    }
}

/// 无焦点候选窗将操作发送给前台编辑器，继续走 TSF 的正常按键路径。
/// 与 candidate_window.rs 的 send_virtual_key 同款（SendInput + scan code）。
unsafe fn send_virtual_key(vk: u8) {
    let scan = MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) as u16;
    let key = |up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk as u16),
                wScan: scan,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    Default::default()
                },
                ..Default::default()
            },
        },
    };
    let inputs = [key(false), key(true)];
    SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
}
