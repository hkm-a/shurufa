//! 随宿主分发的离线中文 OCR。
//!
//! 模型直接编入可执行文件，避免依赖系统语言包、Python 或独立 OCR 程序。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use paddle_ocr_rs::ocr_lite::OcrLite;

const DET_MODEL: &[u8] = include_bytes!("../resources/ocr/ch_PP-OCRv5_mobile_det.onnx");
const CLS_MODEL: &[u8] = include_bytes!("../resources/ocr/ch_ppocr_mobile_v2.0_cls_infer.onnx");
const REC_MODEL: &[u8] = include_bytes!("../resources/ocr/ch_PP-OCRv5_rec_mobile_infer.onnx");
// 识别模型的输出索引 0 为 CTC 空白符；上游词典从索引 1 开始。
const DICT: &str = concat!("\n", include_str!("../resources/ocr/ppocrv5_dict.txt"));
const ONNX_RUNTIME: &[u8] = include_bytes!("../resources/ocr/onnxruntime.dll");

/// 识别一张 BMP，并按视觉阅读顺序整理为可复制文本。
pub fn recognize_bmp(bmp: &[u8]) -> Result<String, String> {
    let image = image::load_from_memory_with_format(bmp, image::ImageFormat::Bmp)
        .map_err(|error| format!("读取 OCR 图片失败：{error}"))?
        .to_rgb8();
    recognize_rgb(&image)
}

/// 将贴图窗口使用的 BGRA 像素转换为 OCR 所需的 RGB 图像。
pub fn recognize_bgra(bgra: &[u8], width: i32, height: i32) -> Result<String, String> {
    let width = u32::try_from(width).map_err(|_| "OCR 图片宽度无效".to_owned())?;
    let height = u32::try_from(height).map_err(|_| "OCR 图片高度无效".to_owned())?;
    let expected = width as usize * height as usize * 4;
    if bgra.len() != expected {
        return Err("OCR 图片像素数据不完整".to_owned());
    }
    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    for pixel in bgra.chunks_exact(4) {
        rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
    }
    let image = image::RgbImage::from_raw(width, height, rgb)
        .ok_or_else(|| "构造 OCR 图片失败".to_owned())?;
    recognize_rgb(&image)
}

/// 识别 RGB 图像，供命令行和贴图窗口复用。
pub fn recognize_rgb(image: &image::RgbImage) -> Result<String, String> {
    ensure_runtime()?;
    let resources = runtime_resource_dir();
    let dictionary = dictionary_path(&resources)?;
    let det_model = model_path(&resources, DET_MODEL, "ch_PP-OCRv5_mobile_det.onnx")?;
    let cls_model = model_path(&resources, CLS_MODEL, "ch_ppocr_mobile_v2.0_cls_infer.onnx")?;
    let rec_model = model_path(&resources, REC_MODEL, "ch_PP-OCRv5_rec_mobile_infer.onnx")?;
    let mut engine = OcrLite::new();
    engine
        .init_models_with_dict(
            det_model.to_string_lossy().as_ref(),
            cls_model.to_string_lossy().as_ref(),
            rec_model.to_string_lossy().as_ref(),
            dictionary.to_string_lossy().as_ref(),
            2,
        )
        .map_err(|error| format!("初始化中文 OCR 模型失败：{error}"))?;
    let result = engine
        .detect_angle_rollback(image, 50, 1280, 0.5, 0.3, 1.6, true, false, 0.72)
        .map_err(|error| format!("中文 OCR 识别失败：{error}"))?;
    let lines = result
        .text_blocks
        .into_iter()
        .filter(|block| !block.text.trim().is_empty() && block.text_score >= 0.35)
        .map(|block| OcrLine {
            left: block
                .box_points
                .iter()
                .map(|point| point.x)
                .min()
                .unwrap_or(0),
            top: block
                .box_points
                .iter()
                .map(|point| point.y)
                .min()
                .unwrap_or(0),
            text: block.text,
        })
        .collect();
    Ok(join_lines(lines))
}

#[derive(Debug, PartialEq, Eq)]
struct OcrLine {
    left: u32,
    top: u32,
    text: String,
}

fn join_lines(mut lines: Vec<OcrLine>) -> String {
    lines.sort_by_key(|line| (line.top, line.left));
    lines
        .into_iter()
        .map(|line| line.text.trim().to_owned())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\r\n")
}

fn dictionary_path(resources: &Path) -> Result<PathBuf, String> {
    let path = resources.join("ppocrv5_dict.txt");
    write_if_changed(&path, DICT.as_bytes())?;
    Ok(path)
}

fn model_path(resources: &Path, model: &[u8], name: &str) -> Result<PathBuf, String> {
    let path = resources.join(name);
    // 模型文件较大，首次落盘后按字节数复用，避免每次 OCR 重写。
    if std::fs::metadata(&path).map(|meta| meta.len()).ok() != Some(model.len() as u64) {
        write_if_changed(&path, model)?;
    }
    Ok(path)
}

fn ensure_runtime() -> Result<(), String> {
    static READY: OnceLock<Result<(), String>> = OnceLock::new();
    READY
        .get_or_init(|| {
            let runtime = runtime_resource_dir().join("onnxruntime.dll");
            write_if_changed(&runtime, ONNX_RUNTIME)?;
            ort::init_from(runtime.to_string_lossy())
                .commit()
                .map(|_| ())
                .map_err(|error| format!("加载内置 ONNX Runtime 失败：{error}"))
        })
        .clone()
}

fn runtime_resource_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("shurufa")
        .join("ocr")
}

fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if std::fs::read(path).ok().as_deref() == Some(bytes) {
        return Ok(());
    }
    let parent = path.parent().ok_or_else(|| "OCR 资源目录无效".to_owned())?;
    std::fs::create_dir_all(parent).map_err(|error| format!("创建 OCR 资源目录失败：{error}"))?;
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, bytes).map_err(|error| format!("写入 OCR 资源失败：{error}"))?;
    std::fs::rename(&temporary, path).map_err(|error| format!("启用 OCR 资源失败：{error}"))
}

#[cfg(test)]
mod tests {
    use super::{join_lines, recognize_rgb, OcrLine, DICT};

    #[test]
    fn 词典首位保留_ctc_空白符() {
        assert!(DICT.starts_with('\n'));
    }

    #[test]
    fn 识别文字按从上到下从左到右排列() {
        let text = join_lines(vec![
            OcrLine {
                left: 30,
                top: 50,
                text: "第二行".into(),
            },
            OcrLine {
                left: 70,
                top: 10,
                text: "右上".into(),
            },
            OcrLine {
                left: 10,
                top: 10,
                text: "左上".into(),
            },
            OcrLine {
                left: 0,
                top: 80,
                text: "  ".into(),
            },
        ]);
        assert_eq!(text, "左上\r\n右上\r\n第二行");
    }

    #[test]
    fn 内置中文模型能识别随仓库提供的样图() {
        let image = image::load_from_memory(include_bytes!(
            "../resources/ocr-test/paddle-ocr-rs-test_1.png"
        ))
        .expect("OCR 样图必须可读取")
        .to_rgb8();
        let text = recognize_rgb(&image).expect("内置中文 OCR 必须可以完成推理");
        assert!(text
            .chars()
            .any(|character| ('\u{4e00}'..='\u{9fff}').contains(&character)));
    }
}
