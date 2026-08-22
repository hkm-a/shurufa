//! 云端语音转写客户端（v1.2 试点）：OpenAI 兼容 /v1/audio/transcriptions。
//!
//! - API Key 只从环境变量读取（SHURUFA_ASR_API_KEY，回退 AGNES_API_KEY），
//!   不落盘、不进日志（项目红线，与 ai_panel 同约定）。
//! - Base URL / 模型来自 options.json（speech.cloud_base_url / cloud_model）。
//! - multipart 由 reqwest 构造，不再手写 boundary/body。

/// 云端转写配置。
#[derive(Debug, Clone)]
pub struct AsrConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
}

impl AsrConfig {
    /// 从 options + 环境变量组装；key 缺失返回 Err。
    pub fn from_options(opts: &shurufa_options::SpeechSettings) -> Result<Self, String> {
        let key = std::env::var("SHURUFA_ASR_API_KEY")
            .or_else(|_| std::env::var("AGNES_API_KEY"))
            .unwrap_or_default();
        if key.trim().is_empty() {
            return Err(
                "缺少 API Key：请设置环境变量 SHURUFA_ASR_API_KEY（或 AGNES_API_KEY）".to_owned(),
            );
        }
        let base_url = if opts.cloud_base_url.trim().is_empty() {
            std::env::var("SHURUFA_ASR_BASE_URL")
                .unwrap_or_else(|_| shurufa_options::default_cloud_base_url())
        } else {
            opts.cloud_base_url.trim().to_owned()
        };
        let model = if opts.cloud_model.trim().is_empty() {
            shurufa_options::default_cloud_model()
        } else {
            opts.cloud_model.trim().to_owned()
        };
        Ok(Self {
            base_url,
            model,
            api_key: key,
        })
    }
}

/// 解析 OpenAI 兼容转写响应 {"text": "..."}；失败返回可展示的错误。
pub fn parse_response(json: &str) -> Result<String, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("转写响应解析失败：{e}"))?;
    v.get("text")
        .and_then(|t| t.as_str())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            format!(
                "转写响应缺少 text 字段：{}",
                json.chars().take(160).collect::<String>()
            )
        })
}

/// 调用云端转写，返回转写文本。超时 60s（录音通常 3-10s，服务端排队可长）。
pub fn transcribe(cfg: &AsrConfig, wav: &[u8]) -> Result<String, String> {
    use reqwest::blocking::multipart::{Form, Part};

    let form = Form::new()
        .text("model", cfg.model.clone())
        .text("language", "zh".to_string())
        .part(
            "file",
            Part::bytes(wav.to_vec())
                .file_name("speech.wav")
                .mime_str("audio/wav")
                .map_err(|e| format!("构造 multipart 失败：{e}"))?,
        );
    let url = format!(
        "{}/audio/transcriptions",
        cfg.base_url.trim_end_matches('/')
    );
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败：{e}"))?;
    let resp = client
        .post(&url)
        .bearer_auth(&cfg.api_key)
        .multipart(form)
        .send()
        .map_err(|e| format!("转写请求失败：{e}"))?;
    let status = resp.status();
    let text = resp.text().map_err(|e| format!("读取转写响应失败：{e}"))?;
    if status != 200 {
        return Err(format!(
            "转写服务返回 {status}：{}",
            text.chars().take(200).collect::<String>()
        ));
    }
    parse_response(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 响应解析_成功与失败() {
        assert_eq!(
            parse_response(r#"{"text":" 你好，世界。 "}"#).unwrap(),
            "你好，世界。"
        );
        assert!(parse_response(r#"{"error":"boom"}"#).is_err());
        assert!(parse_response("not json").is_err());
        assert!(parse_response(r#"{"text":""}"#).is_err());
    }

    #[test]
    fn 配置组装_缺key报错() {
        let opts = shurufa_options::SpeechSettings {
            backend: "cloud".to_owned(),
            ..Default::default()
        };
        std::env::remove_var("SHURUFA_ASR_API_KEY");
        std::env::remove_var("AGNES_API_KEY");
        assert!(AsrConfig::from_options(&opts).is_err());
    }
}
