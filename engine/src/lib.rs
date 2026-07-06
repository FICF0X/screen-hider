//! Shared engine: window enumeration + hide/show via DLL injection.
//!
//! Both the CLI (`injector`) and the GUI (`ui`) call into this crate so the
//! injection logic lives in exactly one place.

use std::ffi::c_void;
use std::path::PathBuf;

use dll_syringe::process::OwnedProcess;
use dll_syringe::Syringe;
use windows::core::{BOOL, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, TRUE};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::System::Threading::{
    IsWow64Process, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindow, GetWindowLongW, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsWindowVisible, SetWindowDisplayAffinity, GWL_EXSTYLE, GW_OWNER,
    WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};

/// A real, user-facing top-level window.
#[derive(Clone, Debug)]
pub struct WindowInfo {
    /// Window handle as an isize (portable across the FFI boundary).
    pub hwnd: isize,
    /// Owning process id.
    pub pid: u32,
    /// Window title.
    pub title: String,
    /// Owning executable name without extension (e.g. "EXCEL", "chrome"). Used
    /// to group windows by application.
    pub app: String,
}

/// Owning executable name (no path, no extension) for a pid, or "" on failure.
unsafe fn process_name(pid: u32) -> String {
    let handle: HANDLE = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
        Ok(h) => h,
        Err(_) => return String::new(),
    };
    let mut buf = [0u16; 260];
    let mut len = buf.len() as u32;
    let ok = QueryFullProcessImageNameW(
        handle,
        PROCESS_NAME_WIN32,
        PWSTR(buf.as_mut_ptr()),
        &mut len,
    )
    .is_ok();
    let _ = CloseHandle(handle);
    if !ok {
        return String::new();
    }
    let full = String::from_utf16_lossy(&buf[..len as usize]);
    full.rsplit(['\\', '/'])
        .next()
        .unwrap_or(&full)
        .trim_end_matches(".exe")
        .trim_end_matches(".EXE")
        .to_string()
}

/// Window class name.
unsafe fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = GetClassNameW(hwnd, &mut buf);
    String::from_utf16_lossy(&buf[..n as usize])
}

/// Is this an "alt-tab" window — a real application window a user would expect
/// to see in a share picker? Mirrors the shell's own heuristic.
unsafe fn is_real_app_window(hwnd: HWND) -> bool {
    if !IsWindowVisible(hwnd).as_bool() {
        return false;
    }
    if GetWindowTextLengthW(hwnd) == 0 {
        return false;
    }

    // Windows composited away (on another virtual desktop, or a background UWP
    // host) report as cloaked. Those are the phantom "Configuración" entries.
    let mut cloaked: u32 = 0;
    let _ = DwmGetWindowAttribute(
        hwnd,
        DWMWA_CLOAKED,
        &mut cloaked as *mut _ as *mut c_void,
        std::mem::size_of::<u32>() as u32,
    );
    if cloaked != 0 {
        return false;
    }

    let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
    // Tool windows are palettes/floats, not app windows.
    if ex & WS_EX_TOOLWINDOW.0 != 0 {
        return false;
    }
    // Owned windows (dialogs) are not top-level apps unless flagged app-window.
    if let Ok(owner) = GetWindow(hwnd, GW_OWNER) {
        if !owner.is_invalid() && ex & WS_EX_APPWINDOW.0 == 0 {
            return false;
        }
    }

    // The desktop shell itself.
    let class = class_name(hwnd);
    if class == "Progman" || class == "WorkerW" {
        return false;
    }

    true
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let out = &mut *(lparam.0 as *mut Vec<WindowInfo>);

    if is_real_app_window(hwnd) {
        let len = GetWindowTextLengthW(hwnd);
        let mut buf = vec![0u16; (len + 1) as usize];
        let read = GetWindowTextW(hwnd, &mut buf);
        let title = String::from_utf16_lossy(&buf[..read as usize]);

        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));

        out.push(WindowInfo {
            hwnd: hwnd.0 as isize,
            pid,
            title,
            app: process_name(pid),
        });
    }
    TRUE
}

/// List real application windows, grouped by app (sorted by app then title).
pub fn list_windows() -> Vec<WindowInfo> {
    let mut out: Vec<WindowInfo> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut out as *mut _ as isize));
    }
    out.sort_by(|a, b| {
        a.app
            .to_lowercase()
            .cmp(&b.app.to_lowercase())
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
    out
}

/// Hide/show a window that belongs to THIS process. No injection needed — the
/// affinity API works directly on windows owned by the caller. Used to hide the
/// Screen Hider window itself. Returns true on success.
pub fn set_affinity_local(hwnd: i64, hide: bool) -> bool {
    let h = HWND(hwnd as *mut c_void);
    let affinity = if hide { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
    unsafe { SetWindowDisplayAffinity(h, affinity).is_ok() }
}

/// The first real window owned by the current process (i.e. our own window).
pub fn own_main_window() -> Option<isize> {
    let me = std::process::id();
    list_windows().into_iter().find(|w| w.pid == me).map(|w| w.hwnd)
}

/// Is the target process 32-bit (running under WOW64 on 64-bit Windows)?
fn is_wow64(pid: u32) -> bool {
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut wow = windows::core::BOOL(0);
        let _ = IsWow64Process(handle, &mut wow);
        let _ = CloseHandle(handle);
        wow.as_bool()
    }
}

/// Path to the payload DLL matching the target process' architecture. A 64-bit
/// DLL cannot be injected into a 32-bit process (and vice versa), so we ship
/// both `payload64.dll` and `payload32.dll` next to the executable.
pub fn payload_dll_path(pid: u32) -> PathBuf {
    let mut dir = std::env::current_exe().unwrap_or_default();
    dir.pop();
    let name = if is_wow64(pid) {
        "payload32.dll"
    } else {
        "payload64.dll"
    };
    dir.join(name)
}

/// Hide (`hide = true`) or restore (`hide = false`) a window from screen capture.
///
/// Injects payload.dll into the owning process and calls `set_visibility`
/// remotely, since `SetWindowDisplayAffinity` must run inside the owner process.
/// Returns `Ok(true)` when the affinity change succeeded.
///
/// Note: the payload DLL stays loaded in the target after the call (its RPC
/// runtime pins it). That is fine — the window's affinity persists regardless,
/// and end users never rebuild the DLL. We deliberately do NOT eject it:
/// dll-syringe panics ("ejected module survived") when ejecting a payload DLL
/// whose RPC runtime is still active.
pub fn set_hidden(pid: u32, hwnd: isize, hide: bool) -> Result<bool, String> {
    let process =
        OwnedProcess::from_pid(pid).map_err(|e| format!("open process {pid}: {e}"))?;
    let syringe = Syringe::for_process(process);

    let dll = payload_dll_path(pid);
    if !dll.exists() {
        return Err(format!("payload DLL not found at {}", dll.display()));
    }

    // find_or_inject reuses the module if the DLL is already loaded in the target.
    let module = syringe
        .find_or_inject(&dll)
        .map_err(|e| format!("inject {}: {e}", dll.display()))?;

    let remote = unsafe {
        syringe.get_payload_procedure::<fn(i64, bool) -> bool>(module, "set_visibility")
    }
    .map_err(|e| format!("get remote procedure: {e}"))?
    .ok_or_else(|| "procedure 'set_visibility' not found in payload".to_string())?;

    let hwnd_i64 = hwnd as i64;
    remote
        .call(&hwnd_i64, &hide)
        .map_err(|e| format!("remote call failed: {e}"))
}
