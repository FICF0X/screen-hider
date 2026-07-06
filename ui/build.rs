// Embed the application icon into ui.exe (shown in Explorer, the taskbar, and on
// the desktop shortcut). No-op on non-Windows / if the resource compiler is
// unavailable — the in-app window icon still works regardless.
fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("icon.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=icon embed skipped: {e}");
        }
    }
}
