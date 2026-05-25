//! Force the BlueFlame icon onto the native Picture-in-Picture window.
//!
//! WebView2's PiP UI is a top-level window created by `msedgewebview2.exe`
//! (Microsoft's WebView2 host process), not by `blueflame.exe`. Windows
//! pulls the taskbar icon from the owning process by default, so the PiP
//! window ships with the generic WebView2 "screen" icon. Microsoft does
//! not expose any WebView2 API to override it.
//!
//! Workaround (Windows only): when the page emits `enterpictureinpicture`,
//! poll for a new top-level Chromium window (`Chrome_WidgetWin_1`) whose
//! owning PID descends from our process. Once found, copy the icons off
//! our main window via `WM_GETICON` and stamp them onto the PiP window
//! with `WM_SETICON`. Cheap, stable, no external dependencies beyond
//! `windows-sys` (already in the transitive tree).
//!
//! Caveats:
//! - macOS and Linux are no-ops here. On macOS the PiP UI is rendered by
//!   AVKit using the app's bundle icon natively; on Linux PiP isn't really
//!   a thing under WebKit2GTK in the same form. So nothing to do.
//! - Resilient to startup race: polls every 100 ms for up to 2 s. The PiP
//!   window almost always materializes within a single tick.
//! - If WebView2 later updates the icon back to its default, we lose. In
//!   practice this doesn't happen - WebView2 sets the icon synchronously
//!   at window-create time and never touches it again.

#[cfg(not(target_os = "windows"))]
pub fn try_restamp_pip_icon(_app: &tauri::AppHandle) {}

#[cfg(target_os = "windows")]
pub fn try_restamp_pip_icon(app: &tauri::AppHandle) {
    use tauri::Manager;
    let Some(main) = app.get_window("main") else {
        return;
    };
    let handle = match main.hwnd() {
        Ok(h) => h,
        Err(_) => return,
    };
    // tauri's HWND is `windows::Win32::Foundation::HWND` (newtype around
    // *mut c_void). We store as isize for Send across the worker thread,
    // then cast back to the windows-sys HWND type on the receiving side.
    let main_hwnd_raw: isize = handle.0 as isize;
    std::thread::spawn(move || {
        windows_impl::poll_and_restamp(main_hwnd_raw);
    });
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use std::time::Duration;

    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetParent, GetWindowRect, GetWindowThreadProcessId,
        IsWindowVisible, SendMessageW, ICON_BIG, ICON_SMALL, WM_GETICON, WM_SETICON,
    };

    /// Total time we'll keep looking for the PiP window before giving up.
    /// PiP almost always shows within ~150 ms of the JS call; 2 s is the
    /// generous upper bound so a sluggish system still gets the icon.
    const POLL_ATTEMPTS: u32 = 20;
    const POLL_INTERVAL: Duration = Duration::from_millis(100);
    /// Refresh the process snapshot on the first attempt and then again
    /// every few ticks. Refresh is cheap enough that we could do it every
    /// tick, but the PID we're looking for is almost certainly already in
    /// the snapshot we took on attempt 0.
    const REFRESH_EVERY: u32 = 4;

    pub fn poll_and_restamp(main_hwnd_raw: isize) {
        let main_hwnd = main_hwnd_raw as HWND;
        let our_pid = std::process::id();
        let mut sys = System::new();
        for attempt in 0..POLL_ATTEMPTS {
            if attempt % REFRESH_EVERY == 0 {
                sys.refresh_processes_specifics(
                    ProcessesToUpdate::All,
                    true,
                    ProcessRefreshKind::nothing(),
                );
            }
            if let Some(pip_hwnd) = find_pip_window(main_hwnd, our_pid, &sys) {
                stamp(main_hwnd, pip_hwnd);
                return;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    struct EnumCtx<'a> {
        main_hwnd: HWND,
        our_pid: u32,
        sys: &'a System,
        found: Option<HWND>,
    }

    unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> i32 {
        let ctx = unsafe { &mut *(lparam as *mut EnumCtx) };
        if hwnd == ctx.main_hwnd {
            return 1;
        }
        if unsafe { IsWindowVisible(hwnd) } == 0 {
            return 1;
        }
        if !unsafe { GetParent(hwnd) }.is_null() {
            return 1;
        }
        // Chromium's standard top-level window class. Tauri's own
        // main window uses a different class ("Window Class" or the
        // Tauri-generated one), so this filter discriminates the PiP
        // surface from BlueFlame's own chrome.
        let mut cls = [0u16; 64];
        let n = unsafe { GetClassNameW(hwnd, cls.as_mut_ptr(), cls.len() as i32) };
        if n <= 0 {
            return 1;
        }
        let cls_str = String::from_utf16_lossy(&cls[..n as usize]);
        if cls_str != "Chrome_WidgetWin_1" {
            return 1;
        }
        // Sanity guard: PiP windows are small (typically 400 x 240),
        // but a tab-detached webview popup or a tooltip stub could
        // also use this class. Anything with a tiny or absurd rect is
        // skipped.
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
            return 1;
        }
        let w = rect.right - rect.left;
        let h = rect.bottom - rect.top;
        if w < 80 || h < 60 {
            return 1;
        }
        // Process tree check: only stamp windows owned by a process
        // descended from our blueflame.exe (= a WebView2 host child of
        // ours). Skips any unrelated Chromium app the user may have
        // open elsewhere on the desktop.
        let mut pid: u32 = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if pid == 0 {
            return 1;
        }
        if !pid_descends_from(ctx.sys, pid, ctx.our_pid) {
            return 1;
        }
        ctx.found = Some(hwnd);
        0
    }

    fn find_pip_window(main_hwnd: HWND, our_pid: u32, sys: &System) -> Option<HWND> {
        let mut ctx = EnumCtx {
            main_hwnd,
            our_pid,
            sys,
            found: None,
        };
        unsafe {
            EnumWindows(Some(enum_cb), &mut ctx as *mut _ as LPARAM);
        }
        ctx.found
    }

    /// Walk parent PIDs up from `pid` looking for `target_pid`. Cap the
    /// walk so a future bug in sysinfo (cycle, runaway parent chain)
    /// can't spin forever. Real Windows process trees are shallow - 8
    /// hops is far more than any plausible browser nesting depth.
    fn pid_descends_from(sys: &System, pid: u32, target_pid: u32) -> bool {
        let target = Pid::from_u32(target_pid);
        let mut current = Some(Pid::from_u32(pid));
        for _ in 0..8 {
            let Some(p) = current else {
                return false;
            };
            if p == target {
                return true;
            }
            current = sys.process(p).and_then(|pr| pr.parent());
        }
        false
    }

    /// Copy the icons off our main window and onto the PiP window. Using
    /// WM_GETICON on our own window means we reuse whatever icon Tauri
    /// already loaded for us - no need to find the embedded resource or
    /// re-load from disk, and we automatically pick up any future icon
    /// changes (e.g. dark/light variants).
    fn stamp(main_hwnd: HWND, target_hwnd: HWND) {
        unsafe {
            let small = SendMessageW(main_hwnd, WM_GETICON, ICON_SMALL as WPARAM, 0);
            let big = SendMessageW(main_hwnd, WM_GETICON, ICON_BIG as WPARAM, 0);
            if small != 0 {
                SendMessageW(target_hwnd, WM_SETICON, ICON_SMALL as WPARAM, small);
            }
            if big != 0 {
                SendMessageW(target_hwnd, WM_SETICON, ICON_BIG as WPARAM, big);
            }
        }
    }
}
