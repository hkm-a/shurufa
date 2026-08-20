//! 候选窗的 DirectComposition GPU 直渲后端（瀑布链最高优先级）。
//!
//! ## 模块定位
//!
//! 渲染瀑布链：`DComp → D2D → GDI`。本模块把候选窗整帧画进一张 premultiplied
//! alpha 的 flip-model DXGI swapchain（DComp composition swapchain），由
//! IDCompositionVisual 挂上目标窗口，DWM 在合成阶段直接融合，完全绕开
//! `ID2D1HwndRenderTarget` 的 GDI 兼容 present 路径：
//! - 逐帧 vsync（`Present(1, 0)`），不再有 D2D hwnd target 的即时 present；
//! - 圆角直接画进纹理（premultiplied alpha），`WS_EX_NOREDIRECTIONBITMAP`
//!   让内容不经 GDI redirection surface 中转，不需要 `WS_EX_LAYERED`；
//! - 翻页/选位变化只重画一次纹理；纯位移类的后续演化（行 visual
//!   `SetOffsetX2/Y2`）可做到零纹理重建（当前每帧一整张纹理，留好了
//!   per-row visual 槽位与 `commit_offsets` 接口）。
//!
//! ## 降级链（任何一步失败都静默退回下一级，绝不 panic）
//!
//! 1. `try_init`（CandidateUi::new 时调用）：建 ID2D1DeviceContext GPU 路径
//!    （D3D11CreateDevice + IDXGIFactory2::CreateSwapChainForComposition +
//!    DCompositionCreateDevice）。任一步 HRESULT 失败 → `Backend::Failed`,
//!    本会话落到 D2D。
//! 2. 首帧 paint：`ensure_frame_state` 建 DCompositionTargetForHwnd +
//!    root visual + swapchain。失败 → 当帧返回 false，上层落 D2D/GDI。
//! 3. `EndDraw` D2DERR_RECREATE_TARGET / `Present` DXGI_ERROR_DEVICE_REMOVED
//!    （TDR/驱动重置）→ 清空帧状态，下帧惰性整链重建；失败则永久 Failed。
//!
//! ## 资源划分
//!
//! - `GpuCore`（会话级，不随窗口尺寸变）：D2D/DWrite factories + D3D11
//!   device + IDXGIFactory2 + IDCompositionDevice。
//! - `FrameState`（随 HWND + 尺寸 + DPI 走）：swapchain(2 buffers, flip,
//!   premult)、DComp target/root、dcomp-viz D2D target、ID2D1Bitmap1 包装
//!   的 backbuffer。WM_SIZE / WM_DPICHANGED 只标脏，下一帧重画时惰性重建
//!   （与 D2D 后端同一拍，避免 WM_SIZE 回调里干重活）。

use std::cell::RefCell;

use windows::core::{w, Interface, HRESULT};
use windows::Win32::Foundation::{HMODULE, HWND, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Bitmap1, ID2D1Device, ID2D1DeviceContext, ID2D1Factory1,
    ID2D1SolidColorBrush, D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET,
    D2D1_BITMAP_PROPERTIES1, D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_DRAW_TEXT_OPTIONS_CLIP,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_REGULAR,
    DWRITE_MEASURING_MODE_GDI_NATURAL, DWRITE_TEXT_METRICS,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, IDXGIDevice, IDXGIFactory2, IDXGISurface, IDXGISwapChain1,
    DXGI_CREATE_FACTORY_FLAGS, DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1,
    DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};

use crate::candidate_window::PaintView;
use crate::debug_log;
use crate::skin::Skin;

// ---------------------------------------------------------------------------
// 后端状态机（thread-local，与候选窗同 UI 线程）
// ---------------------------------------------------------------------------

thread_local! {
    static BACKEND: RefCell<Backend> = const { RefCell::new(Backend::Pending) };
}

// GpuCore 是会话级 GPU 资源包（数百字节）；它始终在 thread_local 槽里，
// 与零大小的 Pending/Failed 同枚举时按大变体分配。Box 会引入每帧解引用
// 与额外堆分配，对渲染热路径无益——保留直存并显式豁免该 lint。
#[allow(clippy::large_enum_variant)]
enum Backend {
    /// 尚未尝试初始化
    Pending,
    /// 会话级 GPU 资源就绪；帧级资源随 HWND 惰性
    Ready(GpuCore),
    /// 初始化失败或 TDR 后重建失败：本会话永久退回 D2D，不反复重试
    Failed,
}

/// 单纯探测（测试旁路用）：D3D11 + D2D1 device context 能否建起来。
/// 测试里 mock 这个函数以验证瀑布选路；生产路径由 try_init 落实际资源。
pub fn probe_dcomp_available() -> bool {
    let result = std::panic::catch_unwind(|| unsafe { init_gpu_core().is_some() });
    result.unwrap_or(false)
}

/// 瀑布选路（单独成函数以便 cfg(test) 注入伪探测）。
/// dcomp_ok = true → DComp；false 且 d2d_ok → D2D；都 false → Gdi（最后兜底，
/// GDI 路径在 candidate_window.rs 中是无外部依赖的纯软件绘制，恒可用）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendKind {
    DComp,
    D2D,
    Gdi,
}

pub fn choose_backend_with(dcomp_ok: bool, d2d_ok: bool) -> BackendKind {
    if dcomp_ok {
        BackendKind::DComp
    } else if d2d_ok {
        BackendKind::D2D
    } else {
        BackendKind::Gdi
    }
}

/// 生产路径选路：真实探测，与 `crate::candidate_window::current_backend_kind`
/// 一拍（那边同样走 try_init 后的状态，不重复探测硬件）。
#[allow(dead_code)] // 由 choose_backend_for_window 消费；测试也直接调
pub fn choose_backend() -> BackendKind {
    choose_backend_with(
        probe_dcomp_available(),
        crate::candidate_window_d2d::is_enabled(),
    )
}

#[allow(dead_code)] // 若干部件仅 RAII 持有随 FrameState 一并释放
struct GpuCore {
    d2d_factory: ID2D1Factory1,
    dwrite: IDWriteFactory,
    d3d: ID3D11Device,
    d3d_ctx: ID3D11DeviceContext,
    dcomp: IDCompositionDevice,
    dxgi_factory: IDXGIFactory2,
    fmt_cand: Option<IDWriteTextFormat>,
    fmt_sub: Option<IDWriteTextFormat>,
    /// (dpi, cand_font_px, sub_font_px)
    fmt_key: (u32, f32, f32),
    /// 帧级状态：swapchain + DComp target/viz + backbuffer viz target。
    /// 尺寸/DPI/HWND 任一变化即整体重建。
    frame: Option<FrameState>,
}

#[allow(dead_code)] // target 与 root 用于 RAII 持有
struct FrameState {
    hwnd: HWND,
    size: D2D_SIZE_U,
    dpi: u32,
    swap: IDXGISwapChain1,
    _d3d_surface: Option<IDXGISurface>,
    target: IDCompositionTarget,
    root: IDCompositionVisual,
    viz_target: ID2D1DeviceContext,
    viz_bitmap: ID2D1Bitmap1,
    brushes: Option<Brushes>,
    brush_key: Skin,
    /// 每行一个 DComp visual 的槽位：当前翻页/高亮变化都走整帧重画；
    /// 留给后续做"零纹理重建"的位移动画（SetOffsetY2 / AddVisual）。
    row_visuals: Vec<IDCompositionVisual>,
}

struct Brushes {
    background: ID2D1SolidColorBrush,
    highlight: ID2D1SolidColorBrush,
    /// 悬停底（blend(highlight, background, 40%)），随皮肤重建
    hover: ID2D1SolidColorBrush,
    text: ID2D1SolidColorBrush,
    preedit: ID2D1SolidColorBrush,
    /// 音节分段交替色（blend(preedit, text, 28%)）；与 D2D 后端同公式。
    preedit_alt: ID2D1SolidColorBrush,
    label: ID2D1SolidColorBrush,
    sb_track: ID2D1SolidColorBrush,
}

// ---------------------------------------------------------------------------
// 对外接口（与 candidate_window.rs 的 wnd_proc 调度对接）
// ---------------------------------------------------------------------------

/// 首帧前预热（候选窗构造时与 D2D try_init 并排调用一次）。
/// 成功 → Backend::Ready；失败 → Failed 并记日志（不 panic）。
pub fn try_init() {
    BACKEND.with_borrow_mut(|b| {
        if !matches!(*b, Backend::Pending) {
            return;
        }
        *b = match unsafe { init_gpu_core() } {
            Some(core) => Backend::Ready(core),
            None => {
                debug_log("candidate_dcomp: gpu init failed; waterfall to D2D");
                Backend::Failed
            }
        };
    });
}

/// 顶层调度用：false 时调用方必须落 D2D → GDI。
pub fn is_enabled() -> bool {
    BACKEND.with_borrow(|b| matches!(*b, Backend::Ready(_)))
}

/// WM_PAINT 调度。true = 本帧已由 DComp 完整画出并 Present；
/// false = 任何故障，调用方须立刻落下一级后端重画，画面不丢。
pub fn paint(hwnd: HWND, rc: &RECT, view: &PaintView) -> bool {
    if !is_enabled() {
        return false;
    }
    // 与 D2D 后端同一纪律：整帧 catch_unwind，绝不让 panic 逃逸进 TSF 窗口过程。
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        BACKEND.with_borrow_mut(|b| match &mut *b {
            Backend::Ready(core) => core.paint_frame(hwnd, rc, view),
            _ => false,
        })
    }));
    match result {
        Ok(ok) => ok,
        Err(_) => {
            debug_log("candidate_dcomp: panic in frame; fallback this frame");
            false
        }
    }
}

/// WM_SIZE / WM_DPICHANGED：帧级资源标脏，下一帧惰性重建。
pub fn notify_resize() {
    BACKEND.with_borrow_mut(|b| {
        if let Backend::Ready(core) = &mut *b {
            core.frame = None;
        }
    });
}

/// 皮肤主题热切换：brushes 下帧重建（与 D2D 后端同名接口对齐）。
pub fn notify_skin_changed() {
    BACKEND.with_borrow_mut(|b| {
        if let Backend::Ready(core) = &mut *b {
            if let Some(frame) = &mut core.frame {
                frame.brushes = None;
            }
        }
    });
}

/// WM_DESTROY：释放全部 COM/GPU 资源；下一次创建候选窗重新走 Pending → Ready。
pub fn shutdown() {
    BACKEND.with_borrow_mut(|b| {
        *b = Backend::Pending;
    });
}

// ---------------------------------------------------------------------------
// 会话级 GPU 资源
// ---------------------------------------------------------------------------

unsafe fn init_gpu_core() -> Option<GpuCore> {
    // D3D11 设备：硬件优先，WARP 兜底——只是为了"能不能开 GPU 管线"的探测，
    // 候选窗这种纹理量 WARP 也完全跑得动。
    let flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;
    let mut d3d: Option<ID3D11Device> = None;
    let mut d3d_ctx: Option<ID3D11DeviceContext> = None;
    let hw = D3D11CreateDevice(
        None,
        D3D_DRIVER_TYPE_HARDWARE,
        HMODULE::default(),
        flags,
        None,
        D3D11_SDK_VERSION,
        Some(&mut d3d),
        None,
        Some(&mut d3d_ctx),
    );
    if hw.is_err() || d3d.is_none() {
        d3d = None;
        d3d_ctx = None;
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_WARP,
            HMODULE::default(),
            flags,
            None,
            D3D11_SDK_VERSION,
            Some(&mut d3d),
            None,
            Some(&mut d3d_ctx),
        )
        .ok()?;
    }
    let d3d = d3d?;
    let d3d_ctx = d3d_ctx?;

    let dxgi_dev: IDXGIDevice = d3d.cast().ok()?;
    let dxgi_factory: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).ok()?;
    let d2d_factory: ID2D1Factory1 =
        D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).ok()?;
    let d2d_dev: ID2D1Device = d2d_factory.CreateDevice(&dxgi_dev).ok()?;
    // 建一次 viz device context 即验证整链通；帧级每窗口再新建议便于随 DPI 换。
    let _probe_ctx: ID2D1DeviceContext = d2d_dev
        .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)
        .ok()?;
    let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi_dev).ok()?;
    let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).ok()?;

    Some(GpuCore {
        d2d_factory,
        dwrite,
        d3d,
        d3d_ctx,
        dcomp,
        dxgi_factory,
        fmt_cand: None,
        fmt_sub: None,
        fmt_key: (0, 0.0, 0.0),
        frame: None,
    })
}

// ---------------------------------------------------------------------------
// 帧级资源与绘制
// ---------------------------------------------------------------------------

impl GpuCore {
    fn paint_frame(&mut self, hwnd: HWND, rc: &RECT, v: &PaintView) -> bool {
        if !self.ensure_frame(hwnd, rc, v.dpi) {
            return false;
        }
        if !self.ensure_formats(v) {
            return false;
        }
        // take 出来避免 self 借用打架（与 D2D 后端同一手法）
        let Some(mut frame) = self.frame.take() else {
            return false;
        };
        if !unsafe { ensure_brushes(&mut frame, v.skin) } {
            self.frame = Some(frame);
            return false;
        }
        let ok = render_and_present(self, &mut frame, rc, v);
        if ok {
            self.frame = Some(frame);
        }
        // 失败路径：frame 丢弃，下一帧 ensure_frame 惰性整链重建（TDR 友好）
        ok
    }

    fn ensure_frame(&mut self, hwnd: HWND, rc: &RECT, dpi: u32) -> bool {
        let w = (rc.right - rc.left).max(0) as u32;
        let h = (rc.bottom - rc.top).max(0) as u32;
        if w == 0 || h == 0 {
            return false;
        }
        // 窗口隐藏时 GetClientRect 尺寸可能还在；由调用处保证只在可见时 paint。
        let size = D2D_SIZE_U {
            width: w,
            height: h,
        };
        if let Some(f) = &self.frame {
            if f.hwnd == hwnd && f.size == size && f.dpi == dpi {
                return true;
            }
        }
        self.frame = None; // 旧资源随 RAII 释放
        self.frame = unsafe { self.create_frame(hwnd, size, dpi) };
        self.frame.is_some()
    }

    unsafe fn create_frame(&self, hwnd: HWND, size: D2D_SIZE_U, dpi: u32) -> Option<FrameState> {
        // ---- swapchain：flip-model + premultiplied alpha + composition ----
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: size.width,
            Height: size.height,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: false.into(),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            Flags: 0,
        };
        let swap: IDXGISwapChain1 = self
            .dxgi_factory
            .CreateSwapChainForComposition(&self.d3d, &desc, None)
            .ok()?;

        // ---- DComp target + root visual，内容 = swapchain ----
        let target = self.dcomp.CreateTargetForHwnd(hwnd, true).ok()?;
        let root = self.dcomp.CreateVisual().ok()?;
        root.SetContent(&swap).ok()?;
        target.SetRoot(&root).ok()?;

        // ---- dcomp-viz D2D device context + 包一层 backbuffer 当 bitmap target ----
        let dxgi_dev: IDXGIDevice = self.d3d.cast().ok()?;
        let d2d_dev: ID2D1Device = self.d2d_factory.CreateDevice(&dxgi_dev).ok()?;
        let dc: ID2D1DeviceContext = d2d_dev
            .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)
            .ok()?;
        // 布局值（scale 后）即物理像素：D2D 1:1 解释（96 DPI = DIP 与物理相同）
        dc.SetDpi(96.0, 96.0);

        let backbuffer: IDXGISurface = swap.GetBuffer(0).ok()?;
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: windows::Win32::Graphics::Direct2D::Common::D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT(0), // 让 D2D 沿用 surface 自身格式
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            colorContext: std::mem::ManuallyDrop::new(None),
        };
        let viz_bitmap: ID2D1Bitmap1 = dc
            .CreateBitmapFromDxgiSurface(&backbuffer, Some(&props))
            .ok()?;
        dc.SetTarget(&viz_bitmap);

        Some(FrameState {
            hwnd,
            size,
            dpi,
            swap,
            _d3d_surface: Some(backbuffer),
            target,
            root,
            viz_target: dc,
            viz_bitmap,
            brushes: None,
            brush_key: Skin::default(),
            row_visuals: Vec::new(),
        })
    }

    fn ensure_formats(&mut self, v: &PaintView) -> bool {
        let key = (v.dpi, v.cand_font_h as f32, v.sub_font_h as f32);
        if self.fmt_key == key && self.fmt_cand.is_some() && self.fmt_sub.is_some() {
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
}

unsafe fn ensure_brushes(frame: &mut FrameState, skin: Skin) -> bool {
    if frame.brushes.is_some() && frame.brush_key == skin {
        return true;
    }
    let t = &frame.viz_target;
    let mk = |c: u32| color_from_colorref(c, 1.0);
    let background = t
        .CreateSolidColorBrush(&mk(skin.candidate.background), None)
        .ok();
    let highlight = t
        .CreateSolidColorBrush(&mk(skin.candidate.highlight_background), None)
        .ok();
    let hover_c = crate::candidate_window::blend_colorref(
        skin.candidate.highlight_background,
        skin.candidate.background,
        400,
    );
    let hover = t.CreateSolidColorBrush(&mk(hover_c), None).ok();
    let text = t.CreateSolidColorBrush(&mk(skin.candidate.text), None).ok();
    let preedit = t
        .CreateSolidColorBrush(&mk(skin.candidate.preedit), None)
        .ok();
    // 音节分段交替色（=candidate_window::syllable_segment_colors[1]），派生色不动 skin。
    let preedit_alt_c =
        crate::candidate_window::blend_colorref(skin.candidate.preedit, skin.candidate.text, 280);
    let preedit_alt = t.CreateSolidColorBrush(&mk(preedit_alt_c), None).ok();
    let label = t
        .CreateSolidColorBrush(&mk(skin.candidate.label), None)
        .ok();
    let sb_track_c = crate::skin::scrollbar_colors(&skin).0;
    let sb_track = t.CreateSolidColorBrush(&mk(sb_track_c), None).ok();
    match (
        background,
        highlight,
        hover,
        text,
        preedit,
        preedit_alt,
        label,
        sb_track,
    ) {
        (
            Some(background),
            Some(highlight),
            Some(hover),
            Some(text),
            Some(preedit),
            Some(preedit_alt),
            Some(label),
            Some(sb_track),
        ) => {
            frame.brushes = Some(Brushes {
                background,
                highlight,
                hover,
                text,
                preedit,
                preedit_alt,
                label,
                sb_track,
            });
            frame.brush_key = skin;
            true
        }
        _ => false,
    }
}

/// 画一整帧并 Present。任何一步 Err/RECREATE 都返回 false（外部重建）。
fn render_and_present(core: &GpuCore, frame: &mut FrameState, rc: &RECT, v: &PaintView) -> bool {
    // flip swapchain 的 backbuffer 每帧可能换对象（缓冲轮换）；稳妥做法
    // 是每帧重包 bitmap。代价极小（仅建 ID2D1Bitmap1 包装，不分配显存）。
    unsafe {
        let Ok(backbuffer) = frame.swap.GetBuffer::<IDXGISurface>(0) else {
            return false;
        };
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: windows::Win32::Graphics::Direct2D::Common::D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT(0),
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            // 1:1（布局坐标即物理像素）
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            colorContext: std::mem::ManuallyDrop::new(None),
        };
        let Ok(bitmap) = frame
            .viz_target
            .CreateBitmapFromDxgiSurface(&backbuffer, Some(&props))
        else {
            return false;
        };
        frame.viz_bitmap = bitmap;
    }

    let Some(br) = frame.brushes.take() else {
        return false;
    };
    let (Some(fmt_c), Some(fmt_s)) = (core.fmt_cand.clone(), core.fmt_sub.clone()) else {
        frame.brushes = Some(br);
        return false;
    };

    let dc = &frame.viz_target;
    let end_ok = unsafe {
        dc.SetTarget(&frame.viz_bitmap);
        // 布局值（scale 后）即物理像素：1:1 绘制，不再按窗口 DPI 放大
        //（此前 SetDpi(dpi) 会把内容整体放大 dpi/96 倍，超出窗口被裁剪）。
        dc.SetDpi(96.0, 96.0);
        dc.BeginDraw();
        // premultiplied 语义：透明底色起笔，圆角/抗锯齿边缘天然预乘。
        dc.Clear(Some(&D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }));
        draw_body(dc, rc, v, &br, &fmt_c, &fmt_s, &core.dwrite);
        match dc.EndDraw(None, None) {
            Ok(()) => true,
            Err(e) => {
                const D2DERR_RECREATE_TARGET: HRESULT = HRESULT(0x8899_000Cu32.cast_signed());
                if e.code() == D2DERR_RECREATE_TARGET {
                    debug_log("candidate_dcomp: RECREATE_TARGET; next frame rebuilds");
                } else {
                    debug_log("candidate_dcomp: EndDraw failed; frame aborted");
                }
                false
            }
        }
    };
    frame.brushes = Some(br);
    if !end_ok {
        return false;
    }

    // 提交 DComp tree，然后 flip。DWM 合成器直接消费 swapchain，
    // 无需额外的 GDI redirection（WS_EX_NOREDIRECTIONBITMAP 在
    // candidate_window.rs 建窗时给 DComp 内核置位；无该 flag 也能跑）。
    unsafe {
        if core.dcomp.Commit().is_err() {
            debug_log("candidate_dcomp: Commit failed");
            return false;
        }
        match frame.swap.Present(1, DXGI_PRESENT(0)) {
            // 1 = vsync；OCCLUDED/REMOVED 都视为重建信号
            hr if hr.is_ok() => true,
            hr => {
                debug_log("candidate_dcomp: Present failed; next frame rebuilds");
                let _ = hr;
                false
            }
        }
    }
}

/// 主体绘制：与 D2D 后端逐行对齐（同一份 PaintView 布局槽位），
/// 唯一区别是 target 类型与圆角走 premultiplied 纹理。
unsafe fn draw_body(
    dc: &ID2D1DeviceContext,
    rc: &RECT,
    v: &PaintView,
    br: &Brushes,
    fmt_c: &IDWriteTextFormat,
    fmt_s: &IDWriteTextFormat,
    dwrite: &IDWriteFactory,
) {
    let padding = v.padding as f32;
    let row_h = v.row_h as f32;
    let preedit_h = v.preedit_h as f32;
    let radius = v.skin.metrics.radius.max(0) as f32;
    let win_w = (rc.right - rc.left) as f32;
    let win_h = (rc.bottom - rc.top) as f32;

    // ===== 圆角底：先整圆角矩形铺背景色（premultiplied 纹理直接得到
    // anti-aliased 圆角），DWM 合成时窗口外缘自然裁圆。 =====
    let body = D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F {
            left: 0.0,
            top: 0.0,
            right: win_w,
            bottom: win_h,
        },
        radiusX: if radius > 0.0 { radius } else { 0.0 },
        radiusY: if radius > 0.0 { radius } else { 0.0 },
    };
    if radius > 0.0 {
        dc.FillRoundedRectangle(&body, &br.background);
    } else {
        dc.FillRectangle(&body.rect, &br.background);
    }

    // ===== 预编辑行 =====
    let badge_w = crate::candidate_window::mode_badge_width(v) as f32;
    let preedit_right = (win_w - padding - badge_w).max(padding);
    if v.syllable_breaks.is_empty() {
        draw_text(
            dc,
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
        // 音节分段（同 GDI/D2D 语义）：分隔符槽位画 1px 竖线，两侧段交替色。
        draw_preedit_segmented_dcomp(dwrite, dc, fmt_s, br, v, preedit_h, padding);
    }
    // 右上角模式角标：highlight 底色块 + 反色文字（与 D2D 后端 1:1 对齐）。
    if let Some(text) = v.mode_badge {
        let badge_rect = D2D_RECT_F {
            left: preedit_right,
            top: padding,
            right: win_w - padding,
            bottom: padding + preedit_h,
        };
        dc.FillRectangle(&badge_rect, &br.highlight);
        draw_text(dc, fmt_s, &br.background, text, badge_rect);
    }

    // ===== 候选行 =====
    let label_gap = v.label_gap as f32;
    let hl_pad = v.hl_pad as f32;
    let comment_gap = crate::candidate_window::scale(2, v.dpi) as f32;
    for it in &v.items {
        // 多行面板：行顶随 item.row 偏移（单行恒 0）。
        let row_top = (v.padding + v.preedit_h) as f32 + it.row as f32 * row_h;
        let end =
            it.x as f32 + it.label_w as f32 + label_gap + it.text_w as f32 + it.badge_w as f32;
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
            dc.FillRoundedRectangle(&hl, &br.highlight);
        } else if it.hovered {
            // 悬停（非选中）：浅色底，与选中态区分（见 Brushes.hover）。
            let hv = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: it.x as f32 - hl_pad,
                    top: row_top,
                    right: end + hl_pad,
                    bottom: row_top + row_h,
                },
                radiusX: radius,
                radiusY: radius,
            };
            dc.FillRoundedRectangle(&hv, &br.hover);
        }

        draw_text(
            dc,
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
            dc,
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

        // 候选来源角标（show_candidate_badge）：主文本右侧小字角标。
        if let Some(badge) = it.source_badge {
            let bx =
                it.x as f32 + it.label_w as f32 + label_gap + it.pure_text_w as f32 + comment_gap;
            draw_text(
                dc,
                fmt_s,
                &br.label,
                badge,
                D2D_RECT_F {
                    left: bx,
                    top: row_top,
                    right: bx + it.badge_w as f32,
                    bottom: row_top + row_h,
                },
            );
        }

        if !it.comment.is_empty() {
            let cx = it.x as f32
                + it.label_w as f32
                + label_gap
                + it.pure_text_w as f32
                + it.badge_w as f32
                + comment_gap;
            draw_text(
                dc,
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

    // ===== 翻页滚动条（皮肤开启且多页时）=====
    if crate::candidate_window::scrollbar_active(v) {
        let track_w = crate::candidate_window::scrollbar_width(v);
        let rows = crate::candidate_window::panel_row_count(&v.items);
        let item_w = if rows > 1 {
            rows * v.row_h
        } else {
            v.items
                .iter()
                .map(|it| it.label_w + v.label_gap + it.text_w + it.badge_w + v.hl_pad * 2)
                .max()
                .unwrap_or(crate::candidate_window::scale(96, v.dpi))
        };
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
            dc.FillRectangle(&r(geo.track), &br.sb_track);
            dc.FillRectangle(&r(geo.thumb), &br.highlight);
        }
    }
}

// ---------------------------------------------------------------------------
// 小工具
// ---------------------------------------------------------------------------

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

fn draw_text(
    dc: &ID2D1DeviceContext,
    fmt: &IDWriteTextFormat,
    brush: &ID2D1SolidColorBrush,
    text: &str,
    rect: D2D_RECT_F,
) {
    if text.is_empty() {
        return;
    }
    let wide: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        dc.DrawText(
            &wide,
            fmt,
            &rect,
            brush,
            D2D1_DRAW_TEXT_OPTIONS_CLIP,
            DWRITE_MEASURING_MODE_GDI_NATURAL,
        );
    }
}

// ---------------------------------------------------------------------------
// 音节分段绘制（DComp 路径；与 candidate_window.rs GDI / candidate_window_d2d 同语义）
// ---------------------------------------------------------------------------

/// 用 IDWriteTextLayout 实测 UTF-16 切片宽度（GDI_NATURAL measuring mode，与
/// DrawText 对齐）；失败/空段返回 0。
unsafe fn measure_utf16_dcomp(
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

/// DComp 版 syllable 分段绘制：断点槽位画 1px 竖线，两侧段在
/// (preedit, preedit_alt) 间交替；separator glyph 不入屏。
/// 与 `crate::candidate_window::draw_preedit_segmented`（GDI）和
/// `crate::candidate_window_d2d::draw_preedit_segmented`（D2D）交付同一份布局数据。
unsafe fn draw_preedit_segmented_dcomp(
    factory: &IDWriteFactory,
    dc: &ID2D1DeviceContext,
    fmt_s: &IDWriteTextFormat,
    br: &Brushes,
    v: &crate::candidate_window::PaintView,
    preedit_h: f32,
    padding: f32,
) {
    let wide: Vec<u16> = v.preedit.encode_utf16().collect();
    let n = wide.len();
    let line_top = padding + preedit_h * 0.25;
    let line_bottom = padding + preedit_h * 0.75;

    let mut x = padding;
    let mut seg_start = 0usize;
    for (idx, &bp) in v.syllable_breaks.iter().enumerate() {
        let bp = (bp as usize).min(n);
        if bp < seg_start || bp >= n {
            continue;
        }
        if bp > seg_start {
            let seg = &wide[seg_start..bp];
            let w = measure_utf16_dcomp(factory, fmt_s, seg, preedit_h);
            let brush = if idx % 2 == 0 {
                &br.preedit
            } else {
                &br.preedit_alt
            };
            if !seg.is_empty() {
                dc.DrawText(
                    seg,
                    fmt_s,
                    &D2D_RECT_F {
                        left: x,
                        top: padding,
                        right: x + w,
                        bottom: padding + preedit_h,
                    },
                    brush,
                    windows::Win32::Graphics::Direct2D::D2D1_DRAW_TEXT_OPTIONS_CLIP,
                    DWRITE_MEASURING_MODE_GDI_NATURAL,
                );
            }
            x += w;
        }
        // 分隔符槽位：实测原字符宽度保持槽位与基线对齐；中央画 1px 竖线。
        let sep_w = measure_utf16_dcomp(factory, fmt_s, &wide[bp..bp + 1], preedit_h);
        if sep_w > 0.0 {
            let cx = x + sep_w / 2.0;
            dc.FillRectangle(
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
        let w = measure_utf16_dcomp(factory, fmt_s, seg, preedit_h);
        let brush = if v.syllable_breaks.len().is_multiple_of(2) {
            &br.preedit
        } else {
            &br.preedit_alt
        };
        dc.DrawText(
            seg,
            fmt_s,
            &D2D_RECT_F {
                left: x,
                top: padding,
                right: x + w,
                bottom: padding + preedit_h,
            },
            brush,
            windows::Win32::Graphics::Direct2D::D2D1_DRAW_TEXT_OPTIONS_CLIP,
            DWRITE_MEASURING_MODE_GDI_NATURAL,
        );
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// 探测函数不得 panic：本开发机有 D3D11，返回 true；无 GPU/无 dcomp.dll
    /// 的机器返回 false 也合法——只要是"返回"而不是"崩"。
    #[test]
    fn probe_dcomp_does_not_panic() {
        let _ok = probe_dcomp_available();
        // 走到这里即通过；true/false 均可接受（CI/dev box 都可能没有 DComp）。
    }

    /// 瀑布选路：DComp 探测失败 → D2D 优先；D2D 也挂 → Gdi 兜底。
    #[test]
    fn waterfall_prefers_dcomp_then_d2d_then_gdi() {
        assert_eq!(
            choose_backend_with(true, true),
            BackendKind::DComp,
            "DComp 可用时必须首选"
        );
        assert_eq!(
            choose_backend_with(false, true),
            BackendKind::D2D,
            "DComp 不可用、D2D 可用 → 选 D2D"
        );
        assert_eq!(
            choose_backend_with(false, false),
            BackendKind::Gdi,
            "两级 GPU 后端全挂 → Gdi 兜底"
        );
        // 异常输入也必须是 Gdi（理论上 DComp=true 就不会再往下传 d2d_ok=false，
        // 但选路函数对入参不做假设）。
        assert_eq!(choose_backend_with(true, false), BackendKind::DComp);
    }

    /// 颜色换算与 D2D 后端一致（COLORREF 0x00BBGGRR → premultiplied float）。
    #[test]
    fn colorref_conversion_matches_d2d() {
        let c = color_from_colorref(0x00AA5500, 1.0);
        assert!((c.r - 0.0).abs() < 1e-6);
        assert!((c.g - 85.0 / 255.0).abs() < 1e-6);
        assert!((c.b - 170.0 / 255.0).abs() < 1e-6);
        assert!((c.a - 1.0).abs() < 1e-6);
    }
}
