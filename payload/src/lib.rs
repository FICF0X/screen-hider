//! Payload DLL — this code runs INSIDE the target process after injection.
//!
//! Because it executes within the process that owns the window, the call to
//! `SetWindowDisplayAffinity` is allowed (the API only works on windows owned
//! by the calling process). The injector calls `set_visibility` remotely via
//! dll-syringe's RPC.

use dll_syringe::payload_utils::payload_procedure;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
};

/// Toggle a window's capture visibility.
/// `hwnd_raw` is the HWND as an isize; `hide` = true hides it from screen
/// capture/sharing (WDA_EXCLUDEFROMCAPTURE), false restores it (WDA_NONE).
/// Returns true on success.
/// `hwnd_raw` is fixed-width i64 (not isize) so the value marshals identically
/// whether the target process is 32- or 64-bit.
#[payload_procedure]
fn set_visibility(hwnd_raw: i64, hide: bool) -> bool {
    let hwnd = HWND(hwnd_raw as usize as *mut core::ffi::c_void);
    let affinity = if hide { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
    unsafe { SetWindowDisplayAffinity(hwnd, affinity).is_ok() }
}
