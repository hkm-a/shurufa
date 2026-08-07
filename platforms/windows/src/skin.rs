//! 跨端皮肤 JSON 的 Windows 候选窗适配。
//!
//! 共享文件位于 schemas/shurufa-skin.json。用户可把同名文件放入
//! %APPDATA%\shurufa，或以 SHURUFA_SKIN_PATH 指定开发期文件。

use std::path::PathBuf;

use serde::Deserialize;

/// GDI COLORREF 颜色（0x00BBGGRR）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CandidateColors {
    pub background: u32,
    pub highlight_background: u32,
    pub text: u32,
    pub preedit: u32,
    pub label: u32,
}

impl Default for CandidateColors {
    fn default() -> Self {
        CandidateColors {
            background: 0x00FF_FFFF,
            highlight_background: 0x00E1_EBD6,
            text: 0x0018_1411,
            preedit: 0x00AB_A29A,
            label: 0x0077_9E1B,
        }
    }
}

#[derive(Deserialize)]
struct SkinFile {
    version: u32,
    light: Option<SkinVariant>,
}

#[derive(Deserialize)]
struct SkinVariant {
    candidate: Option<CandidateSection>,
}

#[derive(Deserialize)]
struct CandidateSection {
    background: Option<String>,
    highlight_background: Option<String>,
    text: Option<String>,
    preedit: Option<String>,
    label: Option<String>,
}

/// 从 JSON 文本取 Windows 候选窗颜色；错误与未知版本全部安全回退。
pub fn candidate_colors_from_json(text: &str) -> CandidateColors {
    let Ok(skin) = serde_json::from_str::<SkinFile>(text) else {
        return CandidateColors::default();
    };
    if skin.version != 1 {
        return CandidateColors::default();
    }
    let Some(candidate) = skin.light.and_then(|variant| variant.candidate) else {
        return CandidateColors::default();
    };
    let fallback = CandidateColors::default();
    CandidateColors {
        background: candidate
            .background
            .as_deref()
            .and_then(parse_colorref)
            .unwrap_or(fallback.background),
        highlight_background: candidate
            .highlight_background
            .as_deref()
            .and_then(parse_colorref)
            .unwrap_or(fallback.highlight_background),
        text: candidate
            .text
            .as_deref()
            .and_then(parse_colorref)
            .unwrap_or(fallback.text),
        preedit: candidate
            .preedit
            .as_deref()
            .and_then(parse_colorref)
            .unwrap_or(fallback.preedit),
        label: candidate
            .label
            .as_deref()
            .and_then(parse_colorref)
            .unwrap_or(fallback.label),
    }
}

/// 按 Windows COLORREF 所需的 BGR 排列转换 #RRGGBB 或 #AARRGGBB。
fn parse_colorref(text: &str) -> Option<u32> {
    let hex = text.strip_prefix('#')?;
    let rgb = match hex.len() {
        6 => hex,
        8 => &hex[2..],
        _ => return None,
    };
    let value = u32::from_str_radix(rgb, 16).ok()?;
    let red = (value >> 16) & 0xff;
    let green = (value >> 8) & 0xff;
    let blue = value & 0xff;
    Some(red | (green << 8) | (blue << 16))
}

/// 读取用户覆盖、开发覆盖或部署的默认皮肤。
pub fn load_candidate_colors() -> CandidateColors {
    let Some(path) = skin_path() else {
        return CandidateColors::default();
    };
    let Ok(metadata) = std::fs::metadata(&path) else {
        return CandidateColors::default();
    };
    if metadata.len() > 128 * 1024 {
        return CandidateColors::default();
    }
    std::fs::read_to_string(path)
        .map(|text| candidate_colors_from_json(&text))
        .unwrap_or_default()
}

fn skin_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SHURUFA_SKIN_PATH").map(PathBuf::from) {
        return path.is_file().then_some(path);
    }
    let user = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|root| root.join("shurufa").join("shurufa-skin.json"));
    if user.as_ref().is_some_and(|path| path.is_file()) {
        return user;
    }
    crate::dll_path()
        .parent()
        .map(|dir| dir.join("schemas").join("shurufa-skin.json"))
        .filter(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::{candidate_colors_from_json, CandidateColors};

    #[test]
    fn maps_shared_candidate_colors_to_colorref() {
        let colors = candidate_colors_from_json(
            r##"{
                "version": 1,
                "light": {
                    "candidate": {
                        "background": "#112233",
                        "highlight_background": "#445566",
                        "text": "#778899",
                        "preedit": "#AABBCC",
                        "label": "#DDEEFF"
                    }
                }
            }"##,
        );
        assert_eq!(colors.background, 0x0033_2211);
        assert_eq!(colors.highlight_background, 0x0066_5544);
        assert_eq!(colors.text, 0x0099_8877);
        assert_eq!(colors.preedit, 0x00CC_BBAA);
        assert_eq!(colors.label, 0x00FF_EEDD);
    }

    #[test]
    fn malformed_color_keeps_the_default() {
        let colors = candidate_colors_from_json(
            r##"{"version":1,"light":{"candidate":{"background":"orange"}}}"##,
        );
        assert_eq!(colors, CandidateColors::default());
    }
}
