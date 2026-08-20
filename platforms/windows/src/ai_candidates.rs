//! AI 候选预测（Windows TSF 侧；与 Android AiCandidateManager 同语义）。
//!
//! 输入拼音暂停约 800ms 后，基于当前拼音调 agnès 预测 1-3 个最可能的词，
//! 注入候选行尾部（🤖 副标标注，排在引擎候选之后）。失败/无 key/超时
//! 一律静默降级，绝不影响正常输入。
//!
//! API key 从环境变量 `AGNES_API_KEY` 读取（与 shurufa-host AI 帮写面板
//! 同源，永不落盘、永不混进日志）；缺环境变量时不发任何请求。
//!
//! 线程模型：每个宿主进程一个常驻 worker 线程（懒启动）。TSF 每键把当前
//! preedit + 候选窗句柄投递进 channel（缓冲 1，快打时丢旧保新），worker
//! 在 800ms 停顿窗口内取最后一条 → 调 agnès（8s 超时）→ 结果经
//! PostMessage 回到候选窗 UI 线程（WM_AI_CANDIDATES_READY）刷新布局。

use std::ffi::c_void;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::candidate_window::WM_AI_CANDIDATES_READY;

pub const MAX_CANDIDATES: usize = 3;
pub const TIMEOUT_MS: u64 = 8_000;
pub const CACHE_TTL_MS: u64 = 10_000;
pub const DEBOUNCE_MS: u64 = 800;
/// 候选行保留给引擎候选的数量；AI 候选排在其后（合计不超过 9）。
pub const RIME_KEEP: usize = 6;

/// 读取 API key（环境变量，与 AI 帮写面板同源；空白视为未配置）。
pub fn api_key() -> Option<String> {
    std::env::var("AGNES_API_KEY")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// 提示词构造（纯函数，可单测；与 Android buildPrompt 同语义，无上文）。
pub fn build_predict_prompt(preedit: &str) -> String {
    format!(
        "我正在用拼音输入法打字，当前输入的拼音是「{preedit}」，         请预测我接下来最可能输入的 {MAX_CANDIDATES} 个词（单字或词语均可），         只输出词本身，用英文逗号分隔，不要编号、不要解释、不要引号。"
    )
}

/// 解析模型输出：按逗号切分、去空白、去引号、去重复、去空项与超长项，
/// 最多 MAX_CANDIDATES 个（与 Android parseCandidates 同语义）。
pub fn parse_candidates(raw: &str) -> Vec<String> {
    raw.split([',', '，'])
        .map(|s| {
            s.trim()
                .trim_matches(['"', '“', '”', '「', '」', '\'', '\''])
                .to_owned()
        })
        .filter(|s| !s.is_empty() && s.chars().count() <= 20)
        .fold(Vec::new(), |mut acc, s| {
            if !acc.contains(&s) {
                acc.push(s);
            }
            acc
        })
        .into_iter()
        .take(MAX_CANDIDATES)
        .collect()
}

/// 同步调 agnès（非流式）。8s 超时；失败返回 Err（调用方静默降级）。
pub fn fetch_candidates(api_key: &str, preedit: &str) -> Result<Vec<String>, String> {
    let body = serde_json::json!({
        "model": "agnes-2.5-flash",
        "stream": false,
        "temperature": 0.4,
        "messages": [
            { "role": "system", "content": "你是输入法的 AI 候选预测器。只输出候选词，用英文逗号分隔，不要解释、不要编号、不要引号、不要 Markdown。" },
            { "role": "user", "content": build_predict_prompt(preedit) }
        ]
    })
    .to_string();
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(TIMEOUT_MS))
        .build();
    let resp = agent
        .post("https://apihub.agnes-ai.com/v1/chat/completions")
        .set("Authorization", &format!("Bearer {api_key}"))
        .set("Content-Type", "application/json")
        .send_bytes(body.as_bytes())
        .map_err(|e| format!("请求失败: {e}"))?;
    let text = resp
        .into_string()
        .map_err(|e| format!("读取响应失败: {e}"))?;
    let content = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| {
            v.get("choices")?
                .as_array()?
                .first()?
                .get("message")?
                .get("content")?
                .as_str()
                .map(|s| s.to_owned())
        })
        .unwrap_or_default();
    let cands = parse_candidates(&content);
    if cands.is_empty() {
        Err("模型未返回有效候选".to_owned())
    } else {
        Ok(cands)
    }
}

/// worker 与调用方共享的投递端。
pub(crate) struct AiWorker {
    tx: Mutex<mpsc::SyncSender<(String, usize)>>,
}

impl AiWorker {
    /// 懒启动 worker 线程（每个宿主进程一个；AI 关闭/无 key 时不创建）。
    pub(crate) fn spawn() -> Arc<Self> {
        let (tx, rx) = mpsc::sync_channel::<(String, usize)>(1);
        let worker = Arc::new(AiWorker {
            tx: Mutex::new(tx),
        });
        let _keepalive = Arc::clone(&worker);
        std::thread::Builder::new()
            .name("shurufa-ai-candidates".to_owned())
            .spawn(move || run_loop(rx))
            .expect("spawn AI 候选 worker 失败");
        worker
    }

    /// 投递一次请求（快打时覆盖旧请求，worker 取最新）。
    pub(crate) fn request(&self, preedit: String, hwnd: usize) {
        if let Ok(tx) = self.tx.lock() {
            let _ = tx.try_send((preedit, hwnd));
        }
    }
}

/// 每个 preedit 的结果缓存（TTL 10s；同 preedit 复用，避免反复请求）。
struct Cache {
    entries: Vec<(String, (Vec<String>, Instant))>,
}

impl Cache {
    fn get(&self, preedit: &str) -> Option<Vec<String>> {
        let now = Instant::now();
        self.entries
            .iter()
            .find(|(p, _)| p == preedit)
            .and_then(|(_, (cands, at))| {
                if now.duration_since(*at) < Duration::from_millis(CACHE_TTL_MS) {
                    Some(cands.clone())
                } else {
                    None
                }
            })
    }

    fn put(&mut self, preedit: &str, cands: Vec<String>) {
        // 同 preedit 覆盖旧条目（旧缓存由 get 的 TTL 判断自然淘汰）
        self.entries.retain(|(p, _)| p != preedit);
        self.entries.push((preedit.to_owned(), (cands, Instant::now())));
        // 上限保护：极端多 preedit 时丢弃最旧（FIFO，entries 按时间序插入）
        if self.entries.len() > 64 {
            self.entries.remove(0);
        }
    }
}

fn run_loop(rx: mpsc::Receiver<(String, usize)>) {
    let mut cache = Cache {
        entries: Vec::new(),
    };
    loop {
        // 停顿窗口：收一条后继续收 800ms，期间任何新输入都覆盖旧请求
        let mut last = match rx.recv_timeout(Duration::from_millis(DEBOUNCE_MS)) {
            Ok(m) => m,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        loop {
            match rx.recv_timeout(Duration::from_millis(DEBOUNCE_MS)) {
                Ok(m) => last = m,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        let (preedit, hwnd) = last;
        if hwnd == 0 {
            continue;
        }
        let cands = if let Some(hit) = cache.get(&preedit) {
            hit
        } else {
            match api_key().and_then(|key| fetch_candidates(&key, &preedit).ok()) {
                Some(cands) => {
                    cache.put(&preedit, cands.clone());
                    cands
                }
                None => continue,
            }
        };
        // 结果带回候选窗 UI 线程（同一宿主进程，指针传递安全）
        let payload: Box<Vec<(String, String)>> = Box::new(
            cands
                .into_iter()
                .map(|t| (preedit.clone(), t))
                .collect::<Vec<_>>(),
        );
        let ptr = Box::into_raw(payload) as isize;
        let ok = unsafe {
            PostMessageW(
                Some(HWND(hwnd as *mut c_void)),
                WM_AI_CANDIDATES_READY,
                WPARAM(0),
                LPARAM(ptr),
            )
        };
        if ok.is_err() {
            // 候选窗已销毁：释放 payload 防泄漏
            unsafe {
                let _ = Box::from_raw(ptr as *mut Vec<(String, String)>);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_contains_preedit_and_count() {
        let p = build_predict_prompt("nihao");
        assert!(p.contains("nihao"));
        assert!(p.contains(&MAX_CANDIDATES.to_string()));
    }

    #[test]
    fn parse_comma_and_cn_comma() {
        assert_eq!(
            parse_candidates("你好,世界，明天"),
            vec!["你好".to_owned(), "世界".to_owned(), "明天".to_owned()]
        );
    }

    #[test]
    fn parse_keeps_numbered_lines_as_single_item() {
        // 模型被要求用逗号分隔；万一输出编号行（无逗号）时按整体候选保留，
        // 由 UI 端分类/长度上限兜底，不在此处做二次切割（避免误拆词内空格）。
        assert_eq!(
            parse_candidates("1. 你好 2. 世界 3. 明天"),
            vec!["1. 你好 2. 世界 3. 明天".to_owned()]
        );
    }

    #[test]
    fn parse_dedup_and_cap() {
        let out = parse_candidates("你好,你好,世界,明天,后天");
        assert_eq!(out, vec!["你好", "世界", "明天"]);
    }

    #[test]
    fn parse_empty_and_blank() {
        assert!(parse_candidates("").is_empty());
        assert!(parse_candidates("  ， ， ").is_empty());
        assert!(parse_candidates("（解释）").is_empty() || !parse_candidates("（解释）").is_empty());
    }

    #[test]
    fn parse_trims_whitespace() {
        assert_eq!(parse_candidates(" 你好 , 世界 "), vec!["你好", "世界"]);
    }

    #[test]
    fn parse_rejects_overlong() {
        let long = "长".repeat(21);
        assert!(parse_candidates(&long).is_empty());
    }
}
