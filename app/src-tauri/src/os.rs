//! Windows integration the player's data depends on (ADR 0020): the
//! computer stays awake during an install or an update, and Windows waits
//! for the servers to stop cleanly before it shuts down or logs off.
//! On Linux (ADR 0031) a logind inhibitor keeps it awake; at shutdown the
//! servers get SIGTERM from systemd and stop cleanly on their own.
//!
//! The only module of the app allowed `unsafe` (Win32 FFI): every block has
//! a SAFETY comment.
#![allow(unsafe_code)]

/// Keeps the computer from sleeping while it lives (a power request:
/// tied to the handle, not to a thread).
pub struct KeepAwake {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
    /// `systemd-inhibit ... cat`: holds the lock while `cat` waits on its
    /// stdin, which closes when we drop it, or when the launcher dies.
    #[cfg(target_os = "linux")]
    inhibitor: std::process::Child,
}

// The handle is a plain kernel handle, usable from any thread.
unsafe impl Send for KeepAwake {}
unsafe impl Sync for KeepAwake {}

impl KeepAwake {
    /// `None` when Windows refuses (the work then runs anyway).
    #[cfg(windows)]
    pub fn new(reason: &str) -> Option<Self> {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Power::{
            PowerCreateRequest, PowerRequestSystemRequired, PowerSetRequest,
        };
        use windows_sys::Win32::System::Threading::{
            POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0,
        };
        let mut text = wide(reason);
        let context = REASON_CONTEXT {
            Version: 0, // POWER_REQUEST_CONTEXT_VERSION
            Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
            Reason: REASON_CONTEXT_0 {
                SimpleReasonString: text.as_mut_ptr(),
            },
        };
        // SAFETY: `context` and its string outlive the call (Windows copies
        // the reason); the handle is closed by `Drop`.
        unsafe {
            let handle = PowerCreateRequest(&context);
            if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                return None;
            }
            if PowerSetRequest(handle, PowerRequestSystemRequired) == 0 {
                CloseHandle(handle);
                return None;
            }
            Some(Self { handle })
        }
    }

    /// `None` without logind (`systemd-inhibit` missing or refused).
    #[cfg(target_os = "linux")]
    pub fn new(reason: &str) -> Option<Self> {
        use std::process::{Command, Stdio};
        let mut inhibitor = Command::new("systemd-inhibit")
            .args([
                "--what=sleep:idle",
                "--mode=block",
                "--who=RealmKeeper",
                &format!("--why={reason}"),
                "cat",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        // Refused at once (no logind, no session): the work runs anyway.
        std::thread::sleep(std::time::Duration::from_millis(100));
        match inhibitor.try_wait() {
            Ok(None) => Some(Self { inhibitor }),
            _ => None,
        }
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    pub fn new(_reason: &str) -> Option<Self> {
        None
    }
}

#[cfg(target_os = "linux")]
impl Drop for KeepAwake {
    fn drop(&mut self) {
        drop(self.inhibitor.stdin.take()); // EOF: `cat` ends, the lock goes
        let _ = self.inhibitor.wait();
    }
}

#[cfg(windows)]
impl Drop for KeepAwake {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Power::{PowerClearRequest, PowerRequestSystemRequired};
        // SAFETY: the handle came from `PowerCreateRequest` and is closed once.
        unsafe {
            PowerClearRequest(self.handle, PowerRequestSystemRequired);
            CloseHandle(self.handle);
        }
    }
}

/// While `reason` is set, Windows shows it and waits instead of closing the
/// launcher at shutdown or log-off (the player can still force it); the
/// launcher then stops the servers cleanly on its way out.
#[cfg(windows)]
/// Must run on the window's thread (Windows refuses the call elsewhere):
/// see `commands::update_shutdown_block`.
pub fn block_shutdown(window: &tauri::WebviewWindow, reason: Option<&str>) {
    use windows_sys::Win32::System::Shutdown::{
        ShutdownBlockReasonCreate, ShutdownBlockReasonDestroy,
    };
    let Ok(hwnd) = window.hwnd() else {
        return;
    };
    let hwnd = hwnd.0 as windows_sys::Win32::Foundation::HWND;
    // SAFETY: a live top-level window of this process; the string is
    // copied by Windows.
    unsafe {
        match reason {
            Some(reason) => {
                let text = wide(reason);
                ShutdownBlockReasonCreate(hwnd, text.as_ptr());
            }
            None => {
                ShutdownBlockReasonDestroy(hwnd);
            }
        }
    }
}

#[cfg(not(windows))]
pub fn block_shutdown(_window: &tauri::WebviewWindow, _reason: Option<&str>) {}

#[cfg(windows)]
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
