//! Windows 虚拟键码到 X11 keysym 的翻译（librime 采用 X11 键码约定）。

use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VIRTUAL_KEY, VK_BACK, VK_CAPITAL, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END,
    VK_ESCAPE, VK_HOME, VK_LEFT, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT,
    VK_SPACE, VK_TAB, VK_UP,
};

// librime 修饰键掩码（与 X11 一致）
pub const MASK_SHIFT: i32 = 1 << 0;
pub const MASK_CONTROL: i32 = 1 << 2;
pub const MASK_ALT: i32 = 1 << 3;

// X11 keysym 常量
const XK_BACKSPACE: i32 = 0xff08;
const XK_TAB: i32 = 0xff09;
const XK_RETURN: i32 = 0xff0d;
const XK_ESCAPE: i32 = 0xff1b;
const XK_HOME: i32 = 0xff50;
const XK_LEFT: i32 = 0xff51;
const XK_UP: i32 = 0xff52;
const XK_RIGHT: i32 = 0xff53;
const XK_DOWN: i32 = 0xff54;
const XK_PRIOR: i32 = 0xff55;
const XK_NEXT: i32 = 0xff56;
const XK_END: i32 = 0xff57;
const XK_DELETE: i32 = 0xffff;
const XK_SHIFT_L: i32 = 0xffe1;
const XK_CAPS_LOCK: i32 = 0xffe5;

fn key_pressed(vk: VIRTUAL_KEY) -> bool {
    (unsafe { GetKeyState(vk.0 as i32) } as u16) & 0x8000 != 0
}

/// 读取当前修饰键状态，返回 librime 掩码。
pub fn current_modifiers() -> i32 {
    let mut mask = 0;
    if key_pressed(VK_SHIFT) {
        mask |= MASK_SHIFT;
    }
    if key_pressed(VK_CONTROL) {
        mask |= MASK_CONTROL;
    }
    if key_pressed(VK_MENU) {
        mask |= MASK_ALT;
    }
    mask
}

/// 将虚拟键码翻译为 keysym；无法翻译的键返回 None（交还系统处理）。
/// 可打印字符按美式布局展开 Shift 变体，与 librime 期望一致。
pub fn vk_to_keysym(vk: u32, shift: bool) -> Option<i32> {
    let vk_u16 = vk as u16;
    match VIRTUAL_KEY(vk_u16) {
        VK_SPACE => return Some(0x20),
        VK_BACK => return Some(XK_BACKSPACE),
        VK_TAB => return Some(XK_TAB),
        VK_RETURN => return Some(XK_RETURN),
        VK_ESCAPE => return Some(XK_ESCAPE),
        VK_HOME => return Some(XK_HOME),
        VK_END => return Some(XK_END),
        VK_PRIOR => return Some(XK_PRIOR),
        VK_NEXT => return Some(XK_NEXT),
        VK_LEFT => return Some(XK_LEFT),
        VK_UP => return Some(XK_UP),
        VK_RIGHT => return Some(XK_RIGHT),
        VK_DOWN => return Some(XK_DOWN),
        VK_DELETE => return Some(XK_DELETE),
        VK_SHIFT => return Some(XK_SHIFT_L),
        VK_CAPITAL => return Some(XK_CAPS_LOCK),
        _ => {}
    }

    let ch = match vk {
        // 字母：keysym 即 ASCII 码，区分大小写
        0x41..=0x5A => {
            let base = b'a' + (vk as u8 - 0x41);
            if shift {
                base.to_ascii_uppercase()
            } else {
                base
            }
        }
        // 数字行：Shift 变体按美式布局
        0x30..=0x39 => {
            let digits = b"0123456789";
            let shifted = b")!@#$%^&*(";
            let idx = (vk - 0x30) as usize;
            if shift {
                shifted[idx]
            } else {
                digits[idx]
            }
        }
        // OEM 标点键
        0xBA => tick(shift, b';', b':'),
        0xBB => tick(shift, b'=', b'+'),
        0xBC => tick(shift, b',', b'<'),
        0xBD => tick(shift, b'-', b'_'),
        0xBE => tick(shift, b'.', b'>'),
        0xBF => tick(shift, b'/', b'?'),
        0xC0 => tick(shift, b'`', b'~'),
        0xDB => tick(shift, b'[', b'{'),
        0xDC => tick(shift, b'\\', b'|'),
        0xDD => tick(shift, b']', b'}'),
        0xDE => tick(shift, b'\'', b'"'),
        _ => return None,
    };
    Some(ch as i32)
}

/// 是否是输入法需要接管的未修饰按键。该判断不触发 IPC 或编辑会话，
/// 专供 TSF 的 `OnTestKeyDown` 试探回调使用。
/// `caps_managed` 为 true 时（选项"CapsLock 切英文"开启）接管 CapsLock，
/// 由输入法在 handle_key 里切到英文直输，同时系统不再翻转大写灯。
pub fn is_ime_key(vk: u32, modifiers: i32, caps_managed: bool) -> bool {
    // Shift 不应被输入法接管——系统需要它处理中英文切换之外的场景。
    if vk == VK_SHIFT.0 as u32 {
        return false;
    }
    if vk == VK_CAPITAL.0 as u32 {
        return caps_managed && modifiers & (MASK_CONTROL | MASK_ALT | MASK_SHIFT) == 0;
    }
    modifiers & (MASK_CONTROL | MASK_ALT) == 0
        && vk_to_keysym(vk, modifiers & MASK_SHIFT != 0).is_some()
}

fn tick(shift: bool, normal: u8, shifted: u8) -> u8 {
    if shift {
        shifted
    } else {
        normal
    }
}

#[cfg(test)]
mod tests {
    use super::{is_ime_key, MASK_CONTROL, MASK_SHIFT};

    #[test]
    fn 普通字母由输入法接管而控制组合键直通() {
        assert!(is_ime_key(0x41, 0, false));
        assert!(is_ime_key(0x41, MASK_SHIFT, false));
        assert!(!is_ime_key(0x41, MASK_CONTROL, false));
        assert!(!is_ime_key(0x70, 0, false));
    }

    #[test]
    fn shift不被输入法接管以保证系统切换可用() {
        // VK_SHIFT=0x10
        assert!(!is_ime_key(0x10, 0, false));
        assert!(!is_ime_key(0x10, 0, true));
    }

    #[test]
    fn 大写锁仅在开启切英文选项且无修饰时接管() {
        // VK_CAPITAL=0x14
        assert!(!is_ime_key(0x14, 0, false));
        assert!(is_ime_key(0x14, 0, true));
        assert!(!is_ime_key(0x14, MASK_CONTROL, true));
        assert!(!is_ime_key(0x14, MASK_SHIFT, true));
    }
}
