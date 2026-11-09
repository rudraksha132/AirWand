// Embeds AppIcon.ico as the exe / tray icon (resource id 1). Failure to embed is not fatal.
#[cfg(windows)]
fn main() {
    let mut res = winres::WindowsResource::new();
    res.set_icon("AppIcon.ico");
    let _ = res.compile();
}

#[cfg(not(windows))]
fn main() {}
