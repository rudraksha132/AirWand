//! Tray icon, menu and message loop.
use super::controller::{Controller, RANGES, RANGE_NAMES, SMOOTH_NAMES};
use super::engine::Engine;
use super::receiver::Receiver;
use super::settings::Settings;
use super::{log, native};
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use windows::core::{Result, PCWSTR};
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    NOTIFY_ICON_DATA_FLAGS,
};
use windows::Win32::UI::WindowsAndMessaging::*;

static GAME_HTML: &[u8] = include_bytes!("../../game.html");
const WM_TRAY: u32 = WM_APP + 1;

struct App {
    settings: Arc<Mutex<Settings>>,
    engine: Arc<Engine>,
    receiver: Arc<Receiver>,
    ctrl: Arc<Controller>,
    paused: Arc<AtomicBool>,
    aim_rate: AtomicU64,
    last_aims: AtomicU64,
}

static APP: OnceLock<App> = OnceLock::new();

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn ask_user(name: &str, code: &str) -> bool {
    let text = wide(&format!(
        "This device asked to pair with AirWand. Only allow devices you own.\n\nDevice: {name}\nPairing code: {code}\n\nAllow only if your phone shows the same code."
    ));
    let title = wide(&format!("Allow \u{201C}{name}\u{201D} to control the pointer?"));
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2 | MB_TOPMOST | MB_SETFOREGROUND,
        ) == IDYES
    }
}

pub fn run() -> Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _mutex = CreateMutexW(None, true, windows::core::w!("AirWand.SingleInstance"))?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return Ok(());
        }
        native::tune_process();

        let settings = Arc::new(Mutex::new(Settings::load()));
        let paused = Arc::new(AtomicBool::new(false));
        let engine = Engine::new();
        {
            let s = settings.lock().unwrap();
            engine.set_range(RANGES[s.range_idx.min(2)]);
            engine.set_predict(s.predict);
        }
        engine.start();
        let ctrl = Controller::new(engine.clone(), settings.clone(), paused.clone());

        let c2 = ctrl.clone();
        let receiver = Receiver::new(
            settings.clone(),
            Box::new(move |key, peer_paused, ev| c2.on_event(key, peer_paused, ev)),
            Box::new(|name, code| ask_user(name, code)),
        );
        let r2 = receiver.clone();
        std::thread::spawn(move || r2.start());

        let _ = APP.set(App {
            settings,
            engine,
            receiver,
            ctrl,
            paused,
            aim_rate: AtomicU64::new(0),
            last_aims: AtomicU64::new(0),
        });

        let hinst: HINSTANCE = GetModuleHandleW(None)?.into();
        let class = windows::core::w!("AirWandHidden");
        let wc = WNDCLASSW { lpfnWndProc: Some(wnd_proc), hInstance: hinst, lpszClassName: class, ..Default::default() };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            windows::core::w!("AirWand"),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            None,
            None,
            hinst,
            None,
        )?;

        let nid = tray_data(hwnd, NIF_ICON | NIF_MESSAGE | NIF_TIP, "AirWand");
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        SetTimer(hwnd, 1, 1000, None);
        log::write("started");

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

fn load_icon() -> HICON {
    unsafe {
        let hinst: Option<HINSTANCE> = GetModuleHandleW(None).ok().map(Into::into);
        if let Some(h) = hinst {
            // resource id 1 = AppIcon.ico embedded by build.rs
            if let Ok(i) = LoadIconW(h, PCWSTR(1 as *const u16)) {
                return i;
            }
        }
        LoadIconW(None, IDI_APPLICATION).unwrap_or_default()
    }
}

unsafe fn tray_data(hwnd: HWND, flags: NOTIFY_ICON_DATA_FLAGS, tip: &str) -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW::default();
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid.uFlags = flags;
    nid.uCallbackMessage = WM_TRAY;
    nid.hIcon = load_icon();
    for (i, c) in tip.encode_utf16().take(126).enumerate() {
        nid.szTip[i] = c;
    }
    nid
}

fn tooltip(a: &App) -> String {
    let t = if let Some(e) = a.receiver.error.lock().unwrap().as_ref() {
        let _ = e;
        "AirWand: Bluetooth problem".to_string()
    } else if a.paused.load(Ordering::Relaxed) {
        "AirWand: paused".to_string()
    } else if let Some((name, _)) = a.receiver.active_peer() {
        format!("AirWand: {name}")
    } else if a.receiver.advertising.load(Ordering::Relaxed) {
        "AirWand: waiting for phone".to_string()
    } else {
        "AirWand: starting Bluetooth".to_string()
    };
    t.chars().take(63).collect()
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            let ev = (lp.0 as u32) & 0xFFFF;
            if ev == WM_RBUTTONUP || ev == WM_LBUTTONUP || ev == WM_CONTEXTMENU {
                show_menu(hwnd);
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if let Some(a) = APP.get() {
                let n = a.receiver.aims();
                let prev = a.last_aims.swap(n, Ordering::Relaxed);
                a.aim_rate.store(n.saturating_sub(prev), Ordering::Relaxed);
                let nid = tray_data(hwnd, NIF_TIP, &tooltip(a));
                let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

// menu

const ID_PAUSE: usize = 1;
const ID_CENTER: usize = 2;
const ID_RANGE: usize = 10; // 10..12
const ID_SMOOTH: usize = 20; // 20..23
const ID_PREDICT: usize = 30;
const ID_INVX: usize = 31;
const ID_INVY: usize = 32;
const ID_FORGET: usize = 33;
const ID_ENCRYPT: usize = 34;
const ID_LOG: usize = 35;
const ID_GAME: usize = 36;
const ID_QUIT: usize = 99;

unsafe fn add(menu: HMENU, id: usize, text: &str, checked: bool, enabled: bool) {
    let mut f = MF_STRING;
    if checked {
        f |= MF_CHECKED;
    }
    if !enabled {
        f |= MF_GRAYED;
    }
    let w = wide(text);
    let _ = AppendMenuW(menu, f, id, PCWSTR(w.as_ptr()));
}

unsafe fn sep(menu: HMENU) {
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
}

unsafe fn submenu(menu: HMENU, sub: HMENU, text: &str) {
    let w = wide(text);
    let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, PCWSTR(w.as_ptr()));
}

unsafe fn show_menu(hwnd: HWND) {
    let Some(a) = APP.get() else { return };
    let Ok(menu) = CreatePopupMenu() else { return };
    let s = a.settings.lock().unwrap().clone();
    let paused = a.paused.load(Ordering::Relaxed);
    let peer = a.receiver.active_peer();

    let err = a.receiver.error.lock().unwrap().clone();
    let line = match (&err, &peer) {
        (Some(_), _) => "Bluetooth problem".to_string(),
        (None, Some((name, Some(b)))) => format!("{name} \u{00B7} {b}%"),
        (None, Some((name, None))) => name.clone(),
        (None, None) if a.receiver.advertising.load(Ordering::Relaxed) => "No device connected".to_string(),
        _ => "Bluetooth is starting\u{2026}".to_string(),
    };
    add(menu, 0, &line, false, false);
    if let Some(e) = &err {
        add(menu, 0, e, false, false);
    }
    if peer.is_some() {
        let txt = format!("Aim {}/s \u{00B7} rejected packets {}", a.aim_rate.load(Ordering::Relaxed), a.receiver.rejected());
        add(menu, 0, &txt, false, false);
    }
    sep(menu);
    add(menu, ID_PAUSE, if paused { "Resume" } else { "Pause (free the cursor)" }, false, true);
    add(menu, ID_CENTER, "Center now", false, true);
    sep(menu);

    if let Ok(m) = CreatePopupMenu() {
        for (i, n) in RANGE_NAMES.iter().enumerate() {
            add(m, ID_RANGE + i, n, i == s.range_idx, true);
        }
        submenu(menu, m, "Speed");
    }
    if let Ok(m) = CreatePopupMenu() {
        for (i, n) in SMOOTH_NAMES.iter().enumerate() {
            add(m, ID_SMOOTH + i, n, i == s.smooth_idx, true);
        }
        submenu(menu, m, "Smoothing");
    }
    add(menu, ID_PREDICT, "Predict ahead (hides gaps)", s.predict, true);
    add(menu, ID_INVX, "Invert left/right", s.inv_x, true);
    add(menu, ID_INVY, "Invert up/down", s.inv_y, true);
    sep(menu);
    add(menu, ID_FORGET, &format!("Forget paired devices ({})", s.trusted.len()), false, !s.trusted.is_empty());
    add(menu, ID_ENCRYPT, "Require encrypted link (restart)", s.require_encryption, true);
    add(menu, ID_LOG, "Open log", false, true);
    add(menu, ID_GAME, "Play the target game", false, true);
    sep(menu);
    add(menu, ID_QUIT, "Quit AirWand", false, true);

    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    let _ = SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_RETURNCMD, pt.x, pt.y, 0, hwnd, None);
    let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
    let _ = DestroyMenu(menu);
    if cmd.0 != 0 {
        command(a, cmd.0 as usize, hwnd);
    }
}

fn open_path(p: &std::path::Path) {
    let _ = std::process::Command::new("explorer").arg(p).spawn();
}

unsafe fn command(a: &App, id: usize, hwnd: HWND) {
    let save = |f: &dyn Fn(&mut Settings)| {
        let mut s = a.settings.lock().unwrap();
        f(&mut s);
        s.save();
    };
    match id {
        ID_PAUSE => {
            let now = !a.paused.load(Ordering::Relaxed);
            a.paused.store(now, Ordering::Relaxed);
            a.receiver.set_paused(now);
            a.engine.set_enabled(!now);
        }
        ID_CENTER => a.ctrl.center_all(),
        i if (ID_RANGE..ID_RANGE + 3).contains(&i) => {
            let idx = i - ID_RANGE;
            save(&|s| s.range_idx = idx);
            a.engine.set_range(RANGES[idx]);
        }
        i if (ID_SMOOTH..ID_SMOOTH + 4).contains(&i) => {
            let idx = i - ID_SMOOTH;
            save(&|s| s.smooth_idx = idx);
            a.ctrl.apply_smooth_all();
        }
        ID_PREDICT => {
            save(&|s| s.predict = !s.predict);
            a.engine.set_predict(a.settings.lock().unwrap().predict);
        }
        ID_INVX => save(&|s| s.inv_x = !s.inv_x),
        ID_INVY => save(&|s| s.inv_y = !s.inv_y),
        ID_ENCRYPT => save(&|s| s.require_encryption = !s.require_encryption),
        ID_FORGET => a.receiver.forget_all(),
        ID_LOG => open_path(&log::path()),
        ID_GAME => {
            let p = std::env::temp_dir().join("airwand-game.html");
            if std::fs::write(&p, GAME_HTML).is_ok() {
                open_path(&p);
            }
        }
        ID_QUIT => {
            let nid = tray_data(hwnd, NIF_ICON, "");
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            PostQuitMessage(0);
        }
        _ => {}
    }
}
