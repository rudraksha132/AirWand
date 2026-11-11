//! Thin wrappers over Win32: SendInput, monitors, timers, process tuning.
use airwand::geometry::{Rect, P};
use std::ffi::c_void;
use std::mem::size_of;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{BOOL, HANDLE, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint, HDC, HMONITOR, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::Media::timeBeginPeriod;
use windows::Win32::System::Threading::{
    CreateWaitableTimerExW, GetCurrentProcess, SetPriorityClass, SetProcessInformation, SetWaitableTimer,
    WaitForSingleObject, HIGH_PRIORITY_CLASS, INFINITE, PROCESS_INFORMATION_CLASS, PROCESS_POWER_THROTTLING_STATE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, VIRTUAL_KEY, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK,
    MOUSEEVENTF_WHEEL, MOUSEINPUT, MOUSE_EVENT_FLAGS, GetDoubleClickTime,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN,
};

pub const LEFT_DOWN: u8 = 1;
pub const LEFT_UP: u8 = 2;
pub const RIGHT_DOWN: u8 = 4;
pub const RIGHT_UP: u8 = 8;

/// Call once at start: precise timers, high priority, and no Windows 11 efficiency-mode throttling.
pub fn tune_process() {
    unsafe {
        let _ = timeBeginPeriod(1);
        let _ = SetPriorityClass(GetCurrentProcess(), HIGH_PRIORITY_CLASS);
        // ControlMask: EXECUTION_SPEED (1) | IGNORE_TIMER_RESOLUTION (4); StateMask 0 = opt out of both.
        let state = PROCESS_POWER_THROTTLING_STATE { Version: 1, ControlMask: 0x1 | 0x4, StateMask: 0 };
        let _ = SetProcessInformation(
            GetCurrentProcess(),
            PROCESS_INFORMATION_CLASS(4), // ProcessPowerThrottling
            &state as *const PROCESS_POWER_THROTTLING_STATE as *const c_void,
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        );
    }
}

/// High-resolution waitable timer (about 0.5 ms accuracy); falls back to thread::sleep.
pub struct PreciseTimer {
    h: Option<HANDLE>,
}

impl PreciseTimer {
    pub fn new() -> Self {
        // CREATE_WAITABLE_TIMER_HIGH_RESOLUTION = 0x2, TIMER_ALL_ACCESS = 0x1F0003
        let h = unsafe { CreateWaitableTimerExW(None, PCWSTR::null(), 0x2, 0x1F0003) }.ok();
        Self { h }
    }

    pub fn wait(&self, ms: u64) {
        if let Some(h) = self.h {
            let due: i64 = -(ms as i64) * 10_000;
            unsafe {
                if SetWaitableTimer(h, &due, 0, None, None, BOOL(0)).is_ok() {
                    WaitForSingleObject(h, INFINITE);
                    return;
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

unsafe extern "system" fn monitor_cb(_: HMONITOR, _: HDC, rc: *mut RECT, lp: LPARAM) -> BOOL {
    let v = &mut *(lp.0 as *mut Vec<Rect>);
    let r = *rc;
    v.push(Rect { l: r.left as f32, t: r.top as f32, r: r.right as f32, b: r.bottom as f32 });
    BOOL(1)
}

/// Every monitor, in virtual-desktop pixels (the process is per-monitor DPI aware).
pub fn displays() -> Vec<Rect> {
    let mut v: Vec<Rect> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(monitor_cb), LPARAM(&mut v as *mut Vec<Rect> as isize));
    }
    v
}

pub fn primary() -> Rect {
    unsafe {
        let h = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(h, &mut mi).as_bool() {
            let r = mi.rcMonitor;
            return Rect { l: r.left as f32, t: r.top as f32, r: r.right as f32, b: r.bottom as f32 };
        }
    }
    Rect { l: 0.0, t: 0.0, r: 1920.0, b: 1080.0 }
}

fn virtual_screen() -> Rect {
    unsafe {
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN) as f32;
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN) as f32;
        let w = GetSystemMetrics(SM_CXVIRTUALSCREEN) as f32;
        let h = GetSystemMetrics(SM_CYVIRTUALSCREEN) as f32;
        Rect { l: x, t: y, r: x + w, b: y + h }
    }
}

pub fn cursor_pos() -> Option<P> {
    let mut p = POINT::default();
    unsafe { GetCursorPos(&mut p).ok()? };
    Some((p.x as f32, p.y as f32))
}

pub fn double_click_ms() -> u32 {
    unsafe { GetDoubleClickTime() }
}

fn mouse(dx: i32, dy: i32, data: u32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi: MOUSEINPUT { dx, dy, mouseData: data, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    }
}

fn absolute(p: P) -> INPUT {
    let v = virtual_screen();
    let nx = ((p.0 - v.l) as f64 * 65535.0 / (v.width() as f64 - 1.0).max(1.0)).round().clamp(0.0, 65535.0) as i32;
    let ny = ((p.1 - v.t) as f64 * 65535.0 / (v.height() as f64 - 1.0).max(1.0)).round().clamp(0.0, 65535.0) as i32;
    mouse(nx, ny, 0, MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK)
}

/// Count of SendInput calls in a row that Windows refused (0 events accepted).
static REFUSED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn send(inputs: &[INPUT]) {
    use std::sync::atomic::Ordering;
    let done = unsafe { SendInput(inputs, size_of::<INPUT>() as i32) };
    if done == 0 {
        // Windows refuses input while an administrator window or a UAC prompt is in front, or the screen is locked.
        if REFUSED.fetch_add(1, Ordering::Relaxed) + 1 == 200 {
            super::log::alert("Windows is not letting AirWand move the cursor. This happens while a program running as administrator, a UAC prompt or the lock screen is in front. Click on a normal window, or start AirWand as administrator.");
        }
    } else {
        REFUSED.store(0, Ordering::Relaxed);
    }
}

fn kbd(vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    }
}

/// Types text at the keyboard focus, one UTF-16 unit at a time, whatever the keyboard layout.
pub fn type_text(s: &str) {
    let mut v = Vec::new();
    for u in s.encode_utf16() {
        v.push(kbd(0, u, KEYEVENTF_UNICODE));
        v.push(kbd(0, u, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
    }
    if !v.is_empty() {
        send(&v);
    }
}

/// A special key (codes as in the protocol) with optional ctrl (1), alt (2), shift (4).
pub fn key(code: u8, mods: u8) {
    let (vk, ext): (u16, bool) = match code {
        1 => (0x08, false),
        2 => (0x0D, false),
        3 => (0x09, false),
        4 => (0x1B, false),
        5 => (0x25, true),
        6 => (0x27, true),
        7 => (0x26, true),
        8 => (0x28, true),
        9 => (0x2E, true),
        10 => (0x24, true),
        11 => (0x23, true),
        12 => (0x21, true),
        13 => (0x22, true),
        _ => return,
    };
    let ef = if ext { KEYEVENTF_EXTENDEDKEY } else { KEYBD_EVENT_FLAGS(0) };
    let mods_vk: [(u8, u16); 3] = [(1, 0x11), (2, 0x12), (4, 0x10)];
    let mut v = Vec::new();
    for (bit, m) in mods_vk {
        if mods & bit != 0 {
            v.push(kbd(m, 0, KEYBD_EVENT_FLAGS(0)));
        }
    }
    v.push(kbd(vk, 0, ef));
    v.push(kbd(vk, 0, ef | KEYEVENTF_KEYUP));
    for (bit, m) in mods_vk.iter().rev() {
        if mods & bit != 0 {
            v.push(kbd(*m, 0, KEYEVENTF_KEYUP));
        }
    }
    send(&v);
}

pub fn move_to(p: P) {
    send(&[absolute(p)]);
}

/// Move to p, then press/release the given buttons in one batch.
pub fn button(p: P, flags: u8) {
    let mut v = vec![absolute(p)];
    if flags & LEFT_DOWN != 0 {
        v.push(mouse(0, 0, 0, MOUSEEVENTF_LEFTDOWN));
    }
    if flags & LEFT_UP != 0 {
        v.push(mouse(0, 0, 0, MOUSEEVENTF_LEFTUP));
    }
    if flags & RIGHT_DOWN != 0 {
        v.push(mouse(0, 0, 0, MOUSEEVENTF_RIGHTDOWN));
    }
    if flags & RIGHT_UP != 0 {
        v.push(mouse(0, 0, 0, MOUSEEVENTF_RIGHTUP));
    }
    send(&v);
}

pub fn wheel(delta: i32, horizontal: bool) {
    let flag = if horizontal { MOUSEEVENTF_HWHEEL } else { MOUSEEVENTF_WHEEL };
    send(&[mouse(0, 0, delta as u32, flag)]);
}
