use crate::UserEvent;
use tao::event_loop::EventLoopProxy;
use tray_icon::TrayIconEvent;

#[cfg(target_os = "windows")]
use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicIsize, Ordering},
};

#[cfg(target_os = "windows")]
static HOOK_PROXY: OnceLock<EventLoopProxy<UserEvent>> = OnceLock::new();

#[cfg(target_os = "windows")]
static HOOK_HANDLE: AtomicIsize = AtomicIsize::new(0);

#[cfg(target_os = "windows")]
static TRAY_BOUNDS: Mutex<Option<(f64, f64, f64, f64)>> = Mutex::new(None);

#[cfg(target_os = "windows")]
pub fn install(proxy: EventLoopProxy<UserEvent>) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowsHookExW, WH_MOUSE_LL};

    let _ = HOOK_PROXY.set(proxy);
    // Observe wheel input before taskbar customizations can consume it.
    let hook = unsafe {
        SetWindowsHookExW(
            WH_MOUSE_LL,
            Some(low_level_mouse_hook),
            std::ptr::null_mut(),
            0,
        )
    };
    if hook.is_null() {
        eprintln!("failed to install low-level mouse hook");
    } else {
        HOOK_HANDLE.store(hook as isize, Ordering::Release);
    }
}

#[cfg(target_os = "windows")]
pub fn uninstall() {
    use windows_sys::Win32::UI::WindowsAndMessaging::UnhookWindowsHookEx;

    let hook = HOOK_HANDLE.swap(0, Ordering::AcqRel);
    if hook != 0 {
        unsafe {
            UnhookWindowsHookEx(hook as windows_sys::Win32::UI::WindowsAndMessaging::HHOOK);
        }
    }
}

#[cfg(target_os = "windows")]
pub fn update_tray_bounds(event: &TrayIconEvent) {
    match event {
        TrayIconEvent::Enter { rect, .. } | TrayIconEvent::Move { rect, .. } => {
            if let Ok(mut bounds) = TRAY_BOUNDS.lock() {
                *bounds = Some((
                    rect.position.x,
                    rect.position.y,
                    rect.size.width as f64,
                    rect.size.height as f64,
                ));
            }
        }
        TrayIconEvent::Leave { .. } => {
            if let Ok(mut bounds) = TRAY_BOUNDS.lock() {
                *bounds = None;
            }
        }
        _ => {}
    }
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn low_level_mouse_hook(
    code: i32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::{
        Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_MENU},
        WindowsAndMessaging::{CallNextHookEx, MSLLHOOKSTRUCT, WM_MOUSEHWHEEL, WM_MOUSEWHEEL},
    };

    if code >= 0
        && (wparam == WM_MOUSEWHEEL as usize || wparam == WM_MOUSEHWHEEL as usize)
        && lparam != 0
    {
        // SAFETY: Windows supplies a valid MSLLHOOKSTRUCT for this hook callback.
        let mouse = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
        let delta = ((mouse.mouseData >> 16) & 0xffff) as i16;
        let inside_tray = TRAY_BOUNDS
            .lock()
            .ok()
            .and_then(|bounds| *bounds)
            .is_some_and(|(x, y, width, height)| {
                (mouse.pt.x as f64) >= x
                    && (mouse.pt.y as f64) >= y
                    && (mouse.pt.x as f64) < x + width
                    && (mouse.pt.y as f64) < y + height
            });

        if inside_tray && let Some(proxy) = HOOK_PROXY.get() {
            let ctrl = unsafe { GetAsyncKeyState(VK_CONTROL as i32) < 0 };
            let alt = unsafe { GetAsyncKeyState(VK_MENU as i32) < 0 };
            let _ = proxy.send_event(UserEvent::GlobalWheel { delta, ctrl, alt });
        }
    }

    // SAFETY: forward the input to the next hook in the chain.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}
