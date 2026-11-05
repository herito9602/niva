//! COM initialisation for the worker threads that call COM-based Windows APIs
//! (`ShellExecuteExW`, the Core Audio endpoints).
//!
//! The pool's workers are plain threads: COM is not initialised on them, and a
//! call made without it fails or misbehaves. [`ComGuard`] brackets ONE call
//! with `CoInitializeEx` / `CoUninitialize` on the calling thread, so the
//! thread is left as it was found and nothing is shared between threads.

use windows_sys::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, COINIT_MULTITHREADED, CoInitializeEx,
    CoUninitialize,
};

/// `RPC_E_CHANGED_MODE`: the thread was already initialised in the other mode.
const RPC_E_CHANGED_MODE: i32 = 0x8001_0106_u32 as i32;

/// Whether a `CoInitializeEx` result must be balanced with `CoUninitialize`:
/// only S_OK and S_FALSE (already initialised, same mode) count a reference.
/// `RPC_E_CHANGED_MODE` and every failure do not, but COM is usable after the
/// former (the thread keeps its own mode) so the call may go ahead.
pub fn needs_uninitialize(result: i32) -> bool {
    result == 0 || result == 1
}

/// Whether COM can be used after `CoInitializeEx` returned `result`.
pub fn is_usable(result: i32) -> bool {
    needs_uninitialize(result) || result == RPC_E_CHANGED_MODE
}

pub struct ComGuard {
    uninitialize: bool,
    usable: bool,
}

impl ComGuard {
    /// Single-threaded apartment, the mode the shell wants for `ShellExecute`.
    pub fn apartment() -> Self {
        Self::init(COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE)
    }

    /// Multithreaded apartment, for the audio endpoint calls.
    pub fn multithreaded() -> Self {
        Self::init(COINIT_MULTITHREADED)
    }

    fn init(mode: i32) -> Self {
        // SAFETY: a null reserved pointer is what the API requires.
        let result = unsafe { CoInitializeEx(std::ptr::null(), mode as u32) };
        ComGuard {
            uninitialize: needs_uninitialize(result),
            usable: is_usable(result),
        }
    }

    /// `false` if COM could not be initialised on this thread.
    pub fn is_usable(&self) -> bool {
        self.usable
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.uninitialize {
            // SAFETY: balances the successful `CoInitializeEx` of `init`.
            unsafe { CoUninitialize() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_counted_initialisation_is_balanced() {
        assert!(needs_uninitialize(0), "S_OK");
        assert!(needs_uninitialize(1), "S_FALSE");
        assert!(!needs_uninitialize(RPC_E_CHANGED_MODE));
        assert!(!needs_uninitialize(0x8000_4005_u32 as i32), "E_FAIL");
    }

    #[test]
    fn a_changed_mode_is_still_usable_but_a_failure_is_not() {
        assert!(is_usable(0) && is_usable(1) && is_usable(RPC_E_CHANGED_MODE));
        assert!(!is_usable(0x8000_4005_u32 as i32));
        assert!(!is_usable(0x8007_000E_u32 as i32), "E_OUTOFMEMORY");
    }

    #[test]
    fn a_guard_leaves_the_thread_as_it_found_it() {
        // Each thread starts uninitialised: the first guard counts one
        // reference, a nested one counts another, and both unwind cleanly.
        std::thread::spawn(|| {
            let outer = ComGuard::multithreaded();
            assert!(outer.is_usable());
            {
                let inner = ComGuard::multithreaded();
                assert!(inner.is_usable());
            }
            drop(outer);
            // After the unwind a different mode can be chosen again.
            let after = ComGuard::apartment();
            assert!(after.is_usable());
        })
        .join()
        .unwrap();
    }
}
