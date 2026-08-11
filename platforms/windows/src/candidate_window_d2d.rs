//! 候选窗的 Direct2D 1.x + DirectWrite 渲染后端。
//!
//! ## 模块定位
//!
//! `candidate_window.rs` 保留窗口生命周期、命中测试与完整 GDI 绘制；
//! 本模块在其上提供 GPU 加速路径：亚像素 ClearType、真圆角高亮。
//! 两条路径共享同一份布局结果（`Item::x/label_w/text_w` 全部由 GDI 在
//! show() 计算），保证 D2D/GDI 切换 0 布局漂移。
//!
//! ## 降级链（任何一步失败都静默退回 GDI，绝不 panic）
//!
//! 1. `try_init`（CandidateUi::new 时调用）：仅建 ID2D1Factory +
//!    IDWriteFactory（< 5ms）；失败 → 永久 `Failed`，全会话 GDI。
//! 2. 首次 WM_PAINT：`ensure_target` 建 ID2D1HwndRenderTarget（10-20ms）。
//!    失败 → 当帧返回 false，上层走 GDI BeginPaint/EndPaint 重绘，无黑帧。
//! 3. `EndDraw` 返回 D2DERR_RECREATE_TARGET（TDR/驱动重置）→ 清空
//!    render target，下帧在 ensure_target 处惰性重建；不通知上层，零感知。
//! 4. WM_SIZE / WM_DPICHANGED：仅做轻量标记失效，下一次 paint 时按真实
//!    GetClientRect 重建（不在 WM_SIZE 回调里干重活，避免与系统布局抖动）。
//!
//! ## 设计选择
//!
//! - `ID2D1HwndRenderTarget`：随 HWND 自动管理后台缓冲，无需 flip present
//!   时序；Resize 通过标记失效在下一帧惰性重建（比 WM_SIZE 内同步 Resize
//!   更稳，规避半初始化 target 被并发 WM_PAINT 消费的边角）。
//! - 文本测量：槽位宽度由 GDI 在 show() 计算并写入 PaintData；D2D 端只
//!   DrawText 不再测，杜绝两后端取整差。
//! - 字体/画刷按 (dpi, font_scale, Skin) 惰性 key 重建；同一帧内重复
//!   绘制零分配。

use std::cell::RefCell;

use windows::core::w;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_IGNORE, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Factory, ID2D1HwndRenderTarget, ID2D1SolidColorBrush,
    D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT,
    D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_PRESENT_OPTIONS_NONE, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
    D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_REGULAR,
    DWRITE_MEASURING_MODE_GDI_NATURAL, DWRITE_TEXT_METRICS,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;

use crate::candidate_window::PaintView;
use crate::debug_log;
use crate::skin::Skin;

// ---------------------------------------------------------------------------
// 后端状态机（thread-local，与候选窗同 UI 线程）
// ---------------------------------------------------------------------------

thread_local! {
    static BACKEND: RefCell<Backend> = const { RefCell::new(Backend::Pending) };
}

enum Backend {
    /// 尚未尝试初始化
    Pending,
    /// 工厂已建；target/brushes/formats 按需惰性
    Ready(D2dCore),
    /// 工厂初始化失败：本会话永久 GDI，不反复重试
    Failed,
}

struct D2dCore {
    factory: ID2D1Factory,
    dwrite: IDWriteFactory,
    target: Option<ID2D1HwndRenderTarget>,
    target_size: D2D_SIZE_U,
    target_dpi: u32,
    fmt_cand: Option<IDWriteTextFormat>,
    fmt_sub: Option<IDWriteTextFormat>,
    /// (dpi, cand_font_px, sub_font_px)
    fmt_key: (u32, f32, f32),
    brushes: Option<Brushes>,
    brush_key: Skin,
}

struct Brushes {
    background: ID2D1SolidColorBrush,
    highlight: ID2D1SolidColorBrush,
    text: ID2D1SolidColorBrush,
    preedit: ID2D1SolidColorBrush,
    /// 音节分段交替色（blend(preedit, text, 28%)）；随皮肤重建。
    preedit_alt: ID2D1SolidColorBrush,
    label: ID2D1SolidColorBrush,
    /// 滚动条轨道色（skin.candidate.background 略深一档），随皮肤重建
    sb_track: ID2D1SolidColorBrush,
}

// ---------------------------------------------------------------------------
// 对外接口
// ---------------------------------------------------------------------------

/// 首帧前预热：仅建工厂（< 5ms），结果存入 BACKEND。失败则永久 Failed。
pub fn try_init() {
    BACKEND.with_borrow_mut(|b| {
        if !matches!(*b, Backend::Pending) {
            return;
        }
        *b = match init_factories() {
            Some(core) => Backend::Ready(core),
            None => {
                debug_log("candidate_d2d: factory init failed; permanent GDI fallback");
                Backend::Failed
            }
        };
    });
}

/// 顶层调度用：false 时调用方必须走 GDI 路径。
pub fn is_enabled() -> bool {
    BACKEND.with_borrow(|b| !matches!(*b, Backend::Failed))
}

/// WM_PAINT 调度。true = 已完整画完本帧（调用方仍须 ValidateRect）；
/// false = 任何 D2D 故障，**调用方须立刻落回 GDI 重画一遍**，画面不丢。
pub fn paint(hwnd: HWND, rc: &RECT, view: &PaintView) -> bool {
    if !is_enabled() {
        return false;
    }
    // 保守保守再保守：把整个绘制包进 catch_unwind；一旦泄漏 panic，
    // 也绝不让它逃逸到 TSF 窗口过程。
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        BACKEND.with_borrow_mut(|b| match b {
            Backend::Ready(core) => core.paint_frame(hwnd, rc, view),
            _ => false,
        })
    }));
    match result {
        Ok(ok) => ok,
        Err(_) => {
            debug_log("candidate_d2d: panic in frame; fallback GDI this frame");
            false
        }
    }
}

/// WM_SIZE：标记 target 尺寸失效；下一次 paint 时按 GetClientRect 重建。
pub fn notify_resize() {
    BACKEND.with_borrow_mut(|b| {
        if let Backend::Ready(core) = &mut *b {
            core.target_size = D2D_SIZE_U { width: 0, height: 0 };
        }
    });
}

/// 皮肤主题热切换：brushes 下帧重建。
pub fn notify_skin_changed() {
    BACKEND.with_borrow_mut(|b| {
        if let Backend::Ready(core) = &mut *b {
            core.brushes = None;
        }
    });
}

/// WM_DESTROY：释放全部 COM/GPU 资源，候选窗重建时重新走 Pending → Ready。
pub fn shutdown() {
    BACKEND.with_borrow_mut(|b| {
        *b = Backend::Pending;
    });
}

// ---------------------------------------------------------------------------
// 内部实现
// ---------------------------------------------------------------------------

fn init_factories() -> Option<D2dCore> {
    unsafe {
        let factory: ID2D1Factory =
            D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).ok()?;
        let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).ok()?;
        Some(D2dCore {
            factory,
            dwrite,
            target: None,
            target_size: D2D_SIZE_U { width: 0, height: 0 },
            target_dpi: 0,
            fmt_cand: None,
            fmt_sub: None,
            fmt_key: (0, 0.0, 0.0),
            brushes: None,
            brush_key: Skin::default(),
        })
    }
}

impl D2dCore {
    fn paint_frame(&mut self, hwnd: HWND, rc: &RECT, view: &PaintView) -> bool {
        if !self.ensure_target(hwnd, view.dpi) {
            return false;
        }
        if !self.ensure_formats(view) {
            return false;
        }
        if !self.ensure_brushes(view.skin) {
            return false;
        }
        let Some(target) = self.target.take() else { return false };
        let (br, fmt_c, fmt_s) = match (self.brushes.take(), self.fmt_cand.take(), self.fmt_sub.take()) {
            (Some(b), Some(c), Some(s)) => (b, c, s),
            _ => return false,
        };
        let ok = self.render(&target, rc, view, &br, &fmt_c, &fmt_s);
        // 还回来（不管 EndDraw 结果如何，资源本身可复用）
        self.fmt_sub = Some(fmt_s);
        self.fmt_cand = Some(fmt_c);
        self.brushes = Some(br);
        if ok {
            self.target = Some(target);
        } else {
            // target 已内部标记 dead by EndDraw D2DERR_RECREATE_TARGET
            self.target_size = D2D_SIZE_U { width: 0, height: 0 };
        }
        ok
    }

    #[allow(clippy::too_many_arguments)]
    fn render(
        &self,
        target: &ID2D1HwndRenderTarget,
        rc: &RECT,
        v: &PaintView,
        br: &Brushes,
        fmt_c: &IDWriteTextFormat,
        fmt_s: &IDWriteTextFormat,
    ) -> bool {
        unsafe {
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE);
            target.BeginDraw();
            target.Clear(Some(&br.background.GetColor()));

            let padding = v.padding as f32;
            let row_top = (v.padding + v.preedit_h) as f32;
            let row_h = v.row_h as f32;
            let preedit_h = v.preedit_h as f32;
            let radius = v.skin.metrics.radius.max(0) as f32;

            // ===== 预编辑行 =====
            let badge_w = crate::candidate_window::mode_badge_width(v) as f32;
            let preedit_right = (rc.right as f32 - padding - badge_w).max(padding);
            if v.syllable_breaks.is_empty() {
                draw_text(
                    target,
                    fmt_s,
                    &br.preedit,
                    &v.preedit,
                    D2D_RECT_F {
                        left: padding,
                        top: padding,
                        right: preedit_right,
                        bottom: padding + preedit_h,
                    },
                );
            } else {
                // 与 GDI/DComp 一致的分段视觉：分隔符槽位画 1px 竖线，相邻段交替色。
                draw_preedit_segmented(&self.dwrite, target, fmt_s, br, v, preedit_h, padding);
            }
            // 右上角模式角标：highlight 底色块 + 反色文字（v.mode_badge 为 None 时完全不画）。
            if let Some(text) = v.mode_badge {
                let badge_rect = D2D_RECT_F {
                    left: preedit_right,
                    top: padding,
                    right: rc.right as f32 - padding,
                    bottom: padding + preedit_h,
                };
                target.FillRectangle(&badge_rect, &br.highlight);
                draw_text(
                    target,
                    fmt_s,
                    &br.background,
                    text,
                    badge_rect,
                );
            }

            // ===== 候选行 =====
            let label_gap = v.label_gap as f32;
            let hl_pad = v.hl_pad as f32;
            let comment_gap = crate::candidate_window::scale(2, v.dpi) as f32;
            for it in &v.items {
                let end = it.x as f32 + it.label_w as f32 + label_gap + it.text_w as f32;
                if it.highlighted {
                    let hl = D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: it.x as f32 - hl_pad,
                            top: row_top,
                            right: end + hl_pad,
                            bottom: row_top + row_h,
                        },
                        radiusX: radius,
                        radiusY: radius,
                    };
                    target.FillRoundedRectangle(&hl, &br.highlight);
                }

                draw_text(
                    target,
                    fmt_c,
                    &br.label,
                    &it.label,
                    D2D_RECT_F {
                        left: it.x as f32,
                        top: row_top,
                        right: it.x as f32 + it.label_w as f32,
                        bottom: row_top + row_h,
                    },
                );
                draw_text(
                    target,
                    fmt_c,
                    &br.text,
                    &it.text,
                    D2D_RECT_F {
                        left: it.x as f32 + it.label_w as f32 + label_gap,
                        top: row_top,
                        right: end,
                        bottom: row_top + row_h,
                    },
                );

                if !it.comment.is_empty() {
                    let cx = it.x as f32
                        + it.label_w as f32
                        + label_gap
                        + it.pure_text_w as f32
                        + comment_gap;
                    draw_text(
                        target,
                        fmt_s,
                        &br.label,
                        &it.comment,
                        D2D_RECT_F {
                            left: cx,
                            top: row_top,
                            right: end,
                            bottom: row_top + row_h,
                        },
                    );
                }
            }

            // ===== 翻页滚动条（皮肤开启且多页时；纯视觉，布局宽度已在 show() 预留）=====
            if crate::candidate_window::scrollbar_active(v) {
                let track_w = crate::candidate_window::scrollbar_width(v);
                let item_w = v
                    .items
                    .iter()
                    .map(|it| it.label_w + v.label_gap + it.text_w + v.hl_pad * 2)
                    .max()
                    .unwrap_or(crate::candidate_window::scale(96, v.dpi));
                if let Some(geo) = crate::skin::scrollbar_geo(
                    rc.right,
                    rc.bottom,
                    item_w,
                    v.padding,
                    track_w,
                    v.page.page_no,
                    v.page.total_pages(),
                ) {
                    let r = |g: [i32; 4]| D2D_RECT_F {
                        left: g[0] as f32,
                        top: g[1] as f32,
                        right: g[2] as f32,
                        bottom: g[3] as f32,
                    };
                    target.FillRectangle(&r(geo.track), &br.sb_track);
                    target.FillRectangle(&r(geo.thumb), &br.highlight);
                }
            }

            match target.EndDraw(None, None) {
                Ok(()) => true,
                Err(e) => {
                    const D2DERR_RECREATE_TARGET: i32 = 0x8899_000Cu32.cast_signed();
                    if e.code().0 == D2DERR_RECREATE_TARGET {
                        debug_log("candidate_d2d: RECREATE_TARGET; next frame rebuilds");
                    } else {
                        debug_log("candidate_d2d: EndDraw failed; frame aborted");
                    }
                    false
                }
            }
        }
    }

    fn ensure_target(&mut self, hwnd: HWND, dpi: u32) -> bool {
        unsafe {
            let mut client = RECT::default();
            if windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut client).is_err() {
                return false;
            }
            let w = (client.right - client.left).max(0) as u32;
            let h = (client.bottom - client.top).max(0) as u32;
            if w == 0 || h == 0 {
                return false;
            }
            let size = D2D_SIZE_U { width: w, height: h };

            if self.target.is_some() && self.target_size == size {
                if self.target_dpi != dpi {
                    if let Some(t) = &self.target {
                        t.SetDpi(dpi as f32, dpi as f32);
                        self.target_dpi = dpi;
                        self.fmt_cand = None;
                        self.fmt_sub = None;
                    }
                }
                return true;
            }

            let rt = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_IGNORE,
                },
                dpiX: dpi as f32,
                dpiY: dpi as f32,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let hp = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd,
                pixelSize: size,
                presentOptions: D2D1_PRESENT_OPTIONS_NONE,
            };
            match self.factory.CreateHwndRenderTarget(&rt, &hp) {
                Ok(t) => {
                    // 全部 target 子资源跟着重建
                    self.target = Some(t);
                    self.target_size = size;
                    self.target_dpi = dpi;
                    self.fmt_cand = None;
                    self.fmt_sub = None;
                    self.brushes = None;
                    true
                }
                Err(_) => {
                    self.target = None;
                    self.target_size = D2D_SIZE_U { width: 0, height: 0 };
                    false
                }
            }
        }
    }

    fn ensure_formats(&mut self, v: &PaintView) -> bool {
        let key = (v.dpi, v.cand_font_h as f32, v.sub_font_h as f32);
        let ok = self.fmt_key == key && self.fmt_cand.is_some() && self.fmt_sub.is_some();
        if ok {
            return true;
        }
        unsafe {
            self.fmt_cand = make_format(&self.dwrite, v.cand_font_h as f32);
            self.fmt_sub = make_format(&self.dwrite, v.sub_font_h as f32);
        }
        let done = self.fmt_cand.is_some() && self.fmt_sub.is_some();
        if done {
            self.fmt_key = key;
        }
        done
    }

    fn ensure_brushes(&mut self, skin: Skin) -> bool {
        if self.brushes.is_some() && self.brush_key == skin {
            return true;
        }
        let Some(t) = &self.target else { return false };
        unsafe {
            let mk = |c: u32| color_from_colorref(c, 1.0);
            let background = t.CreateSolidColorBrush(&mk(skin.candidate.background), None).ok();
            let highlight = t
                .CreateSolidColorBrush(&mk(skin.candidate.highlight_background), None)
                .ok();
            let text = t.CreateSolidColorBrush(&mk(skin.candidate.text), None).ok();
            let preedit = t.CreateSolidColorBrush(&mk(skin.candidate.preedit), None).ok();
            // 音节分段交替色（=candidate_window::syllable_segment_colors[1]），派生色不动 skin。
            let preedit_alt_c =
                crate::candidate_window::blend_colorref(skin.candidate.preedit, skin.candidate.text, 280);
            let preedit_alt = t.CreateSolidColorBrush(&mk(preedit_alt_c), None).ok();
            let label = t.CreateSolidColorBrush(&mk(skin.candidate.label), None).ok();
            let sb_track_c = crate::skin::scrollbar_colors(&skin).0;
            let sb_track = t.CreateSolidColorBrush(&mk(sb_track_c), None).ok();
            match (background, highlight, text, preedit, preedit_alt, label, sb_track) {
                (
                    Some(background),
                    Some(highlight),
                    Some(text),
                    Some(preedit),
                    Some(preedit_alt),
                    Some(label),
                    Some(sb_track),
                ) => {
                    self.brushes = Some(Brushes {
                        background,
                        highlight,
                        text,
                        preedit,
                        preedit_alt,
                        label,
                        sb_track,
                    });
                    self.brush_key = skin;
                    true
                }
                _ => false,
            }
        }
    }
}

fn color_from_colorref(c: u32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: (c & 0xff) as f32 / 255.0,
        g: ((c >> 8) & 0xff) as f32 / 255.0,
        b: ((c >> 16) & 0xff) as f32 / 255.0,
        a,
    }
}

unsafe fn make_format(factory: &IDWriteFactory, px: f32) -> Option<IDWriteTextFormat> {
    factory
        .CreateTextFormat(
            w!("Microsoft YaHei UI"),
            None,
            DWRITE_FONT_WEIGHT_REGULAR,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            px,
            w!(""),
        )
        .ok()
}

unsafe fn draw_text(
    target: &ID2D1HwndRenderTarget,
    fmt: &IDWriteTextFormat,
    brush: &ID2D1SolidColorBrush,
    text: &str,
    rect: D2D_RECT_F,
) {
    if text.is_empty() {
        return;
    }
    let wide: Vec<u16> = text.encode_utf16().collect();
    target.DrawText(
        &wide,
        fmt,
        &rect,
        brush,
        D2D1_DRAW_TEXT_OPTIONS_CLIP,
        DWRITE_MEASURING_MODE_GDI_NATURAL,
    );
}

// ---------------------------------------------------------------------------
// 音节分段绘制（D2D 路径；与 candidate_window.rs 的 GDI draw_preedit_segmented 同语义）
// ---------------------------------------------------------------------------

/// 用 IDWriteTextLayout 实测 UTF-16 切片宽度；失败回退 0。
/// GDI_NATURAL measuring mode 与 DrawText 一致，分段推进 x 的累计值 = 整串
/// DrawText 的右侧边缘（零布局漂移）。
unsafe fn measure_utf16(
    factory: &IDWriteFactory,
    fmt: &IDWriteTextFormat,
    wide: &[u16],
    h: f32,
) -> f32 {
    if wide.is_empty() {
        return 0.0;
    }
    let Ok(layout) = factory.CreateTextLayout(wide, fmt, f32::MAX, h) else {
        return 0.0;
    };
    let mut m = DWRITE_TEXT_METRICS::default();
    match layout.GetMetrics(&mut m) {
        Ok(()) => m.width,
        Err(_) => 0.0,
    }
}

/// D2D 版 syllable 分段绘制：断点槽位画 1px 竖线，两侧段在
/// (preedit, preedit_alt) 间交替。separator glyph 不入屏（语义与 GDI 路径一致）。
///
/// `v.syllable_breaks` 已按 UTF-16 码元索引给出分隔符列位；本函数只消费不改计算。
unsafe fn draw_preedit_segmented(
    factory: &IDWriteFactory,
    target: &ID2D1HwndRenderTarget,
    fmt_s: &IDWriteTextFormat,
    br: &Brushes,
    v: &PaintView,
    preedit_h: f32,
    padding: f32,
) {
    let wide: Vec<u16> = v.preedit.encode_utf16().collect();
    let n = wide.len();
    // 竖线覆盖字形高度的中央 50%（与 GDI 路径 (preedit_font_h/2).max(4) 近似对齐）。
    let line_top = padding + preedit_h * 0.25;
    let line_bottom = padding + preedit_h * 0.75;

    let mut x = padding;
    let mut seg_start = 0usize;
    for (idx, &bp) in v.syllable_breaks.iter().enumerate() {
        let bp = (bp as usize).min(n);
        if bp < seg_start || bp >= n {
            continue;
        }
        // 段文本（颜色 mixed-even/odd 轮换）
        if bp > seg_start {
            let seg = &wide[seg_start..bp];
            let w = measure_utf16(factory, fmt_s, seg, preedit_h);
            let brush = if idx % 2 == 0 { &br.preedit } else { &br.preedit_alt };
            if !seg.is_empty() {
                target.DrawText(
                    seg,
                    fmt_s,
                    &D2D_RECT_F {
                        left: x,
                        top: padding,
                        right: x + w,
                        bottom: padding + preedit_h,
                    },
                    brush,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                    DWRITE_MEASURING_MODE_GDI_NATURAL,
                );
            }
            x += w;
        }
        // 分隔符槽位：实测该字符宽度保持基线/槽位对齐；中央 1px 竖线。
        let sep_w = measure_utf16(factory, fmt_s, &wide[bp..bp + 1], preedit_h);
        if sep_w > 0.0 {
            let cx = x + sep_w / 2.0;
            target.FillRectangle(
                &D2D_RECT_F {
                    left: cx,
                    top: line_top,
                    right: cx + 1.0,
                    bottom: line_bottom,
                },
                &br.preedit,
            );
        }
        x += sep_w;
        seg_start = bp + 1;
    }
    // 尾段
    if seg_start < n {
        let seg = &wide[seg_start..];
        let w = measure_utf16(factory, fmt_s, seg, preedit_h);
        let brush = if v.syllable_breaks.len() % 2 == 0 {
            &br.preedit
        } else {
            &br.preedit_alt
        };
        target.DrawText(
            seg,
            fmt_s,
            &D2D_RECT_F {
                left: x,
                top: padding,
                right: x + w,
                bottom: padding + preedit_h,
            },
            brush,
            D2D1_DRAW_TEXT_OPTIONS_CLIP,
            DWRITE_MEASURING_MODE_GDI_NATURAL,
        );
    }
}

// ---------------------------------------------------------------------------
// TODO(future)：升级指针
// - 动效：flip swapchain（D2D1DeviceContext + DCompositionTarget），翻页/
//   选位变化做 60fps 位移+淡入；
// - 排版缓存：候选文本 IDWriteTextLayout LRU，翻页零排版；
// - 阴影：把 ShadowShell 换成 D2D 阴影 effect（Effect API），单窗完成；
// - 诊断开关：SHURUFA_DISABLE_D2D=1 环境变量在 try_init 处短路 Failed。
// ---------------------------------------------------------------------------
