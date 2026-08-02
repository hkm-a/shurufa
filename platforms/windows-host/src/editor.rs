//! 截图标注编辑会话。
//!
//! 编辑器只保有一份冻结底图和 `annotation::EditorState`。窗口中的每次绘制都从
//! 底图重新渲染，确认时才编码 BMP 并交回调用方，避免在拖拽期间破坏原始截图。

use std::cell::RefCell;
use std::io::Cursor;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateCompatibleDC, CreateDIBSection, CreateFontW, CreateSolidBrush, DeleteDC,
    DeleteObject, EndPaint, FillRect, GetDC, InvalidateRect, ReleaseDC, SelectObject, SetBkMode,
    SetTextColor, StretchDIBits, TextOutW, BITMAPINFO, BITMAPINFOHEADER, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS, FF_DONTCARE, HGDIOBJ,
    OUT_DEFAULT_PRECIS, PAINTSTRUCT, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, SetCapture, VK_BACK, VK_CONTROL, VK_ESCAPE, VK_RETURN, VK_Y, VK_Z,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, PostQuitMessage,
    RegisterClassW, ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW, MSG, SW_SHOW, WM_CHAR,
    WM_DESTROY, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WNDCLASSW,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::annotation::{self, Annotation, Color, EditorState, Point};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    Rectangle,
    Arrow,
    Mosaic,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolbarAction {
    Tool(Tool),
    Undo,
    Redo,
    LongScroll,
    Pin,
    Save,
    Confirm,
    Cancel,
}

const TOOLBAR_ROW_HEIGHT: i32 = 40;
const TOOLBAR_HEIGHT: i32 = TOOLBAR_ROW_HEIGHT * 2;
const TOOLBAR_TOP_MARGIN: i32 = 8;
const TOOLBAR_MIN_WIDTH: i32 = 240;

struct EditorWindowState {
    base: image::RgbaImage,
    window_width: i32,
    edits: EditorState,
    tool: Tool,
    start: Option<Point>,
    current: Option<Point>,
    draft_text: String,
    suppress_tool_char: bool,
    confirmed: bool,
    pin_after_copy: bool,
    save_after_copy: bool,
    long_scroll: bool,
}

thread_local! {
    static STATE: RefCell<Option<EditorWindowState>> = const { RefCell::new(None) };
}

/// 编辑完成后的唯一截图出口动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorOutput {
    /// 写入剪贴板，由监听器统一写入历史和同步。
    Copy(Vec<u8>),
    /// 先写入同一剪贴板链路，再把相同图片置顶。
    Pin(Vec<u8>),
    /// 保存 PNG 后仍写入同一剪贴板链路。
    Save(Vec<u8>),
    /// 使用当前区域启动用户手动滚动的长截图会话。
    LongScroll,
}

/// 显示标注窗口。`Ok(None)` 表示用户主动取消；确认和贴图都返回已渲染 BMP。
pub fn edit_bmp(bmp: &[u8]) -> Result<Option<EditorOutput>, String> {
    let base = image::load_from_memory_with_format(bmp, image::ImageFormat::Bmp)
        .map_err(|e| format!("读取截图编辑底图失败：{e}"))?
        .to_rgba8();
    if base.width() == 0 || base.height() == 0 {
        return Err("截图编辑底图尺寸无效".to_owned());
    }
    if base.width() > i32::MAX as u32 || base.height() > i32::MAX as u32 {
        return Err("截图编辑底图尺寸过大".to_owned());
    }
    STATE.with_borrow_mut(|slot| {
        *slot = Some(EditorWindowState {
            window_width: (base.width() as i32).max(TOOLBAR_MIN_WIDTH),
            base,
            edits: EditorState::default(),
            tool: Tool::Rectangle,
            start: None,
            current: None,
            draft_text: String::new(),
            suppress_tool_char: false,
            confirmed: false,
            pin_after_copy: false,
            save_after_copy: false,
            long_scroll: false,
        });
    });
    let result = unsafe { create_and_run_window() };
    let state = STATE.with_borrow_mut(|slot| slot.take());
    result?;
    let Some(state) = state else {
        return Err("截图编辑状态丢失".to_owned());
    };
    if !state.confirmed {
        return Ok(None);
    }
    if state.long_scroll {
        return Ok(Some(EditorOutput::LongScroll));
    }
    let rendered = render_canvas(&state.base, state.edits.annotations())?;
    let mut output = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(rendered)
        .write_to(&mut output, image::ImageFormat::Bmp)
        .map_err(|e| format!("编码标注截图失败：{e}"))?;
    let output = output.into_inner();
    Ok(Some(if state.pin_after_copy {
        EditorOutput::Pin(output)
    } else if state.save_after_copy {
        EditorOutput::Save(output)
    } else {
        EditorOutput::Copy(output)
    }))
}

unsafe fn create_and_run_window() -> Result<(), String> {
    let instance = GetModuleHandleW(PCWSTR::null()).map_err(|e| format!("读取模块失败：{e}"))?;
    let class_name = w!("ShurufaScreenshotEditor");
    let class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wnd_proc),
        hInstance: instance.into(),
        lpszClassName: class_name,
        // 不设光标会导致悬停时一直显示忙碌转圈
        hCursor: windows::Win32::UI::WindowsAndMessaging::LoadCursorW(
            None,
            windows::Win32::UI::WindowsAndMessaging::IDC_ARROW,
        )
        .unwrap_or_default(),
        ..Default::default()
    };
    RegisterClassW(&class);
    let (width, height) = STATE
        .with_borrow(|slot| {
            slot.as_ref()
                .map(|state| (state.window_width, state.base.height() as i32))
        })
        .ok_or_else(|| "截图编辑状态丢失".to_owned())?;
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
        class_name,
        w!("Shurufa 截图标注"),
        WS_POPUP,
        80,
        80,
        width,
        height + TOOLBAR_HEIGHT + TOOLBAR_TOP_MARGIN + 8,
        None,
        None,
        Some(instance.into()),
        None,
    )
    .map_err(|e| format!("创建截图编辑窗口失败：{e}"))?;
    let _ = ShowWindow(hwnd, SW_SHOW);
    let mut message = MSG::default();
    while GetMessageW(&mut message, None, 0, 0).as_bool() {
        let _ = TranslateMessage(&message);
        DispatchMessageW(&message);
    }
    Ok(())
}

fn point_from_lparam(lparam: LPARAM) -> Point {
    let raw = lparam.0 as u32;
    Point {
        x: (raw as u16 as i16) as i32,
        y: ((raw >> 16) as u16 as i16) as i32,
    }
}

fn valid_drag(start: Point, end: Point) -> bool {
    (start.x - end.x).unsigned_abs() >= 4 && (start.y - end.y).unsigned_abs() >= 4
}

fn active_annotation(state: &EditorWindowState) -> Option<Annotation> {
    if matches!(state.tool, Tool::Text) {
        return None;
    }
    let start = state.start?;
    let end = state.current?;
    valid_drag(start, end).then(|| match state.tool {
        Tool::Rectangle => Annotation::Rectangle {
            start,
            end,
            color: Color::RED,
            stroke_width: 3,
        },
        Tool::Arrow => Annotation::Arrow {
            start,
            end,
            color: Color::RED,
            stroke_width: 3,
        },
        Tool::Mosaic => Annotation::Mosaic {
            start,
            end,
            block_size: 12,
        },
        Tool::Text => unreachable!("文字工具不使用拖拽预览"),
    })
}

fn toolbar_buttons(width: i32, height: i32) -> Vec<(RECT, ToolbarAction, &'static str)> {
    let first_row = [
        (36, ToolbarAction::Tool(Tool::Rectangle), "矩"),
        (36, ToolbarAction::Tool(Tool::Arrow), "箭"),
        (36, ToolbarAction::Tool(Tool::Mosaic), "糊"),
        (36, ToolbarAction::Tool(Tool::Text), "字"),
        (42, ToolbarAction::Undo, "撤"),
        (42, ToolbarAction::Redo, "重"),
    ];
    let second_row = [
        (36, ToolbarAction::LongScroll, "长"),
        (36, ToolbarAction::Pin, "贴"),
        (42, ToolbarAction::Save, "存"),
        (42, ToolbarAction::Confirm, "完成"),
        (42, ToolbarAction::Cancel, "取消"),
    ];
    let mut buttons = Vec::new();
    for (row, specs) in [first_row.as_slice(), second_row.as_slice()]
        .into_iter()
        .enumerate()
    {
        let mut left = 10;
        let top = height + TOOLBAR_TOP_MARGIN + row as i32 * TOOLBAR_ROW_HEIGHT;
        for (button_width, action, label) in specs {
            let right = left + button_width;
            let rect = RECT {
                left,
                top,
                right,
                bottom: top + TOOLBAR_ROW_HEIGHT,
            };
            left = right + 5;
            if rect.left < width {
                buttons.push((rect, *action, *label));
            }
        }
    }
    buttons
}

fn toolbar_action_at(width: i32, height: i32, point: Point) -> Option<ToolbarAction> {
    toolbar_buttons(width, height)
        .into_iter()
        .find(|(rect, _, _)| {
            point.x >= rect.left
                && point.x < rect.right
                && point.y >= rect.top
                && point.y < rect.bottom
        })
        .map(|(_, action, _)| action)
}

fn point_in_canvas(width: i32, height: i32, point: Point) -> bool {
    point.x >= 0 && point.y >= 0 && point.x < width && point.y < height
}

unsafe fn paint_toolbar(hdc: windows::Win32::Graphics::Gdi::HDC, state: &EditorWindowState) {
    let width = state.window_width;
    let height = state.base.height() as i32;
    let buttons = toolbar_buttons(width, height);
    let Some((first, _, _)) = buttons.first() else {
        return;
    };
    let Some((last, _, _)) = buttons.last() else {
        return;
    };
    let background_rect = RECT {
        left: first.left - 4,
        top: first.top - 4,
        right: last.right + 4,
        bottom: last.bottom + 4,
    };
    let background = CreateSolidBrush(COLORREF(0x002B2B2B));
    let _ = FillRect(hdc, &background_rect, background);
    let _ = DeleteObject(HGDIOBJ(background.0));
    for (rect, action, label) in buttons {
        let selected = matches!(action, ToolbarAction::Tool(tool) if tool == state.tool);
        let enabled = match action {
            ToolbarAction::Undo => state.edits.can_undo(),
            ToolbarAction::Redo => state.edits.can_redo(),
            _ => true,
        };
        let fill = if selected {
            COLORREF(0x00C77628)
        } else if enabled {
            COLORREF(0x00414141)
        } else {
            COLORREF(0x002F2F2F)
        };
        let brush = CreateSolidBrush(fill);
        let _ = FillRect(hdc, &rect, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));
        let _ = SetTextColor(
            hdc,
            if enabled {
                COLORREF(0x00F5F5F5)
            } else {
                COLORREF(0x00707070)
            },
        );
        let label: Vec<u16> = label.encode_utf16().collect();
        let text_x = rect.left + if label.len() > 1 { 5 } else { 11 };
        let _ = TextOutW(hdc, text_x, rect.top + 13, &label);
    }
    if matches!(state.tool, Tool::Text) {
        let draft: Vec<u16> = format!("文字：{}", state.draft_text)
            .encode_utf16()
            .collect();
        let _ = SetTextColor(hdc, COLORREF(0x00FFFFFF));
        let _ = TextOutW(
            hdc,
            background_rect.right + 10,
            background_rect.top + 12,
            &draft,
        );
    }
}

fn render_canvas(
    base: &image::RgbaImage,
    annotations: &[Annotation],
) -> Result<image::RgbaImage, String> {
    let mut output = base.clone();
    for annotation in annotations {
        match annotation {
            Annotation::Text {
                origin,
                content,
                color,
                font_size,
            } => draw_windows_text(&mut output, *origin, content, *color, *font_size)?,
            _ => output = annotation::render(&output, std::slice::from_ref(annotation)),
        }
    }
    Ok(output)
}

fn draw_windows_text(
    image: &mut image::RgbaImage,
    origin: Point,
    content: &str,
    color: Color,
    font_size: u16,
) -> Result<(), String> {
    if content.is_empty() {
        return Ok(());
    }
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Err("无法创建文字标注的绘制上下文".to_owned());
        }
        let memory = CreateCompatibleDC(Some(screen));
        if memory.is_invalid() {
            let _ = ReleaseDC(None, screen);
            return Err("无法创建文字标注的内存上下文".to_owned());
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: image.width() as i32,
                biHeight: -(image.height() as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = CreateDIBSection(Some(screen), &info, DIB_RGB_COLORS, &mut bits, None, 0)
            .map_err(|error| format!("无法创建文字标注位图：{error}"));
        let result = (|| -> Result<(), String> {
            let bitmap = bitmap?;
            if bits.is_null() {
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
                return Err("文字标注位图没有像素缓冲区".to_owned());
            }
            let mut bgra = image.clone().into_raw();
            for pixel in bgra.chunks_exact_mut(4) {
                pixel.swap(0, 2);
                pixel[3] = 255;
            }
            std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits.cast::<u8>(), bgra.len());
            let previous_bitmap = SelectObject(memory, HGDIOBJ(bitmap.0));
            let font = CreateFontW(
                -(font_size.max(8) as i32),
                0,
                0,
                0,
                400,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                DEFAULT_PITCH.0 as u32 | FF_DONTCARE.0 as u32,
                w!("Microsoft YaHei UI"),
            );
            if font.is_invalid() {
                let _ = SelectObject(memory, previous_bitmap);
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
                return Err("无法创建中文文字标注字体".to_owned());
            }
            let previous_font = SelectObject(memory, HGDIOBJ(font.0));
            let _ = SetBkMode(memory, TRANSPARENT);
            let _ = SetTextColor(
                memory,
                COLORREF(
                    u32::from(color.red)
                        | (u32::from(color.green) << 8)
                        | (u32::from(color.blue) << 16),
                ),
            );
            let text: Vec<u16> = content.encode_utf16().collect();
            let _ = TextOutW(memory, origin.x, origin.y, &text);
            let _ = SelectObject(memory, previous_font);
            let _ = DeleteObject(HGDIOBJ(font.0));
            let mut rgba = std::slice::from_raw_parts(bits.cast::<u8>(), bgra.len()).to_vec();
            for pixel in rgba.chunks_exact_mut(4) {
                pixel.swap(0, 2);
                pixel[3] = 255;
            }
            *image = image::RgbaImage::from_raw(image.width(), image.height(), rgba)
                .ok_or_else(|| "文字标注像素尺寸不匹配".to_owned())?;
            let _ = SelectObject(memory, previous_bitmap);
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            Ok(())
        })();
        let _ = DeleteDC(memory);
        let _ = ReleaseDC(None, screen);
        result
    }
}

unsafe fn paint(hwnd: HWND) {
    let mut paint = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut paint);
    STATE.with_borrow(|slot| {
        let Some(state) = slot.as_ref() else { return };
        let mut annotations = state.edits.annotations().to_vec();
        if let Some(preview) = active_annotation(state) {
            annotations.push(preview);
        }
        let Ok(rendered) = render_canvas(&state.base, &annotations) else {
            return;
        };
        let mut bgra = rendered.into_raw();
        for pixel in bgra.chunks_exact_mut(4) {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: state.base.width() as i32,
                biHeight: -(state.base.height() as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        let _ = StretchDIBits(
            hdc,
            0,
            0,
            state.base.width() as i32,
            state.base.height() as i32,
            0,
            0,
            state.base.width() as i32,
            state.base.height() as i32,
            Some(bgra.as_ptr().cast()),
            &info,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
        let _ = SetBkMode(hdc, TRANSPARENT);
        paint_toolbar(hdc, state);
    });
    let _ = EndPaint(hwnd, &paint);
}

fn select_tool(key: u16) -> Option<Tool> {
    match key {
        0x52 => Some(Tool::Rectangle),
        0x41 => Some(Tool::Arrow),
        0x4d => Some(Tool::Mosaic),
        0x54 => Some(Tool::Text),
        _ => None,
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let point = point_from_lparam(lparam);
            let close = STATE.with_borrow_mut(|slot| {
                if let Some(state) = slot.as_mut() {
                    if let Some(action) =
                        toolbar_action_at(state.window_width, state.base.height() as i32, point)
                    {
                        match action {
                            ToolbarAction::Tool(tool) => {
                                state.tool = tool;
                                if matches!(tool, Tool::Text) {
                                    state.draft_text.clear();
                                }
                                state.suppress_tool_char = false;
                            }
                            ToolbarAction::Undo => {
                                state.edits.undo();
                            }
                            ToolbarAction::Redo => {
                                state.edits.redo();
                            }
                            ToolbarAction::LongScroll => {
                                state.confirmed = true;
                                state.long_scroll = true;
                                return true;
                            }
                            ToolbarAction::Pin => {
                                state.confirmed = true;
                                state.pin_after_copy = true;
                                return true;
                            }
                            ToolbarAction::Save => {
                                state.confirmed = true;
                                state.save_after_copy = true;
                                return true;
                            }
                            ToolbarAction::Confirm => {
                                state.confirmed = true;
                                return true;
                            }
                            ToolbarAction::Cancel => return true,
                        }
                        return false;
                    }
                    if matches!(state.tool, Tool::Text) {
                        if !state.draft_text.is_empty() {
                            state.edits.add(Annotation::Text {
                                origin: point,
                                content: std::mem::take(&mut state.draft_text),
                                color: Color::RED,
                                font_size: 24,
                            });
                        }
                        return false;
                    }
                    if !point_in_canvas(
                        state.base.width() as i32,
                        state.base.height() as i32,
                        point,
                    ) {
                        return false;
                    }
                    state.start = Some(point);
                    state.current = Some(point);
                }
                false
            });
            if close {
                let _ = DestroyWindow(hwnd);
                return LRESULT(0);
            }
            let _ = SetCapture(hwnd);
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            STATE.with_borrow_mut(|slot| {
                if let Some(state) = slot.as_mut().filter(|state| state.start.is_some()) {
                    state.current = Some(point_from_lparam(lparam));
                }
            });
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let point = point_from_lparam(lparam);
            STATE.with_borrow_mut(|slot| {
                if let Some(state) = slot.as_mut() {
                    state.current = Some(point);
                    if let Some(annotation) = active_annotation(state) {
                        state.edits.add(annotation);
                    }
                    state.start = None;
                    state.current = None;
                }
            });
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let key = wparam.0 as u16;
            if key == VK_ESCAPE.0 {
                let _ = DestroyWindow(hwnd);
                return LRESULT(0);
            }
            if key == VK_RETURN.0 {
                STATE.with_borrow_mut(|slot| {
                    if let Some(state) = slot.as_mut() {
                        state.confirmed = true;
                    }
                });
                let _ = DestroyWindow(hwnd);
                return LRESULT(0);
            }
            let control_down = GetKeyState(VK_CONTROL.0 as i32) < 0;
            STATE.with_borrow_mut(|slot| {
                if let Some(state) = slot.as_mut() {
                    if control_down && key == VK_Z.0 {
                        state.edits.undo();
                    } else if control_down && key == VK_Y.0 {
                        state.edits.redo();
                    } else if let Some(tool) = select_tool(key) {
                        // 文字工具输入中，裸字母必须进草稿而非切换工具
                        // （否则文字里永远打不出 r/a/m/t）；此时切换工具
                        // 用 Ctrl+字母 或点击工具条
                        if matches!(state.tool, Tool::Text) && !control_down {
                            return;
                        }
                        state.tool = tool;
                        if matches!(tool, Tool::Text) {
                            state.draft_text.clear();
                            state.suppress_tool_char = true;
                        }
                    }
                }
            });
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_CHAR => {
            let character = char::from_u32(wparam.0 as u32);
            STATE.with_borrow_mut(|slot| {
                if let Some(state) = slot
                    .as_mut()
                    .filter(|state| matches!(state.tool, Tool::Text))
                {
                    if state.suppress_tool_char {
                        state.suppress_tool_char = false;
                    } else if wparam.0 as u16 == VK_BACK.0 {
                        state.draft_text.pop();
                    } else if let Some(character) =
                        character.filter(|character| !character.is_control())
                    {
                        state.draft_text.push(character);
                    }
                }
            });
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 拖拽最小尺寸与区域选择一致() {
        assert!(!valid_drag(Point { x: 1, y: 1 }, Point { x: 3, y: 8 }));
        assert!(valid_drag(Point { x: 1, y: 1 }, Point { x: 5, y: 5 }));
    }

    #[test]
    fn 工具快捷键映射明确且唯一() {
        assert!(matches!(select_tool(b'R' as u16), Some(Tool::Rectangle)));
        assert!(matches!(select_tool(b'A' as u16), Some(Tool::Arrow)));
        assert!(matches!(select_tool(b'M' as u16), Some(Tool::Mosaic)));
        assert!(matches!(select_tool(b'T' as u16), Some(Tool::Text)));
    }

    #[test]
    fn 浮动工具条的鼠标入口没有重复语义() {
        assert_eq!(
            toolbar_action_at(400, 240, Point { x: 20, y: 265 }),
            Some(ToolbarAction::Tool(Tool::Rectangle))
        );
        assert_eq!(
            toolbar_action_at(400, 240, Point { x: 230, y: 265 }),
            Some(ToolbarAction::Redo)
        );
        assert_eq!(
            toolbar_action_at(400, 240, Point { x: 20, y: 313 }),
            Some(ToolbarAction::LongScroll)
        );
        assert_eq!(
            toolbar_action_at(400, 240, Point { x: 70, y: 313 }),
            Some(ToolbarAction::Pin)
        );
        assert_eq!(
            toolbar_action_at(400, 240, Point { x: 100, y: 313 }),
            Some(ToolbarAction::Save)
        );
        assert_eq!(
            toolbar_action_at(400, 240, Point { x: 200, y: 313 }),
            Some(ToolbarAction::Cancel)
        );
        assert_eq!(toolbar_action_at(400, 240, Point { x: 350, y: 80 }), None);
        assert!(!point_in_canvas(400, 240, Point { x: 20, y: 265 }));
    }

    #[test]
    fn 中文文字会被系统字体渲染进导出位图() {
        let mut image = image::RgbaImage::from_pixel(160, 64, image::Rgba([0, 0, 0, 255]));
        draw_windows_text(&mut image, Point { x: 4, y: 4 }, "中文", Color::RED, 24).unwrap();
        assert!(image
            .pixels()
            .any(|pixel| pixel.0[0] > 30 || pixel.0[1] > 30 || pixel.0[2] > 30));
    }
}
