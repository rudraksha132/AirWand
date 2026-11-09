//! Thin wrappers over Win32: SendInput, monitors, timers, process tuning.
use crate::geometry::{Rect, P};
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
    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK,
    MOUSEEVENTF_WHEEL, MOUSEINPUT, MOUSE_EVENT_FLAGS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetDoubleClickTime, GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
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

fn send(inputs: &[INPUT]) {
    unsafe {
        SendInput(inputs, size_of::<INPUT>() as i32);
    }
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
