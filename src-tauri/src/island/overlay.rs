//! A window of ours over a whole monitor, on the island's compositor: the shot's flight, the
//! area selection, the recording's corner marks. Each draws into its root, in the monitor's
//! logical pixels.

use super::comp::{Comp, scale};
use windows::{
    UI::Composition::{ContainerVisual, Desktop::DesktopWindowTarget},
    Win32::{
        Foundation::{HWND, RECT},
        UI::WindowsAndMessaging::DestroyWindow,
    },
    core::Result,
};

pub struct Overlay {
    hwnd: HWND,
    target: Option<DesktopWindowTarget>,
    pub root: ContainerVisual,
    /// The monitor's size in logical pixels.
    pub screen: (f32, f32),
}

impl Overlay {
    /// `monitor` is in physical pixels, `k` is its scale; the window is closed on failure too.
    pub fn open(
        comp: &Comp,
        create: fn(RECT) -> Result<HWND>,
        monitor: RECT,
        k: f32,
    ) -> Result<Self> {
        let hwnd = create(monitor)?;
        let connected = comp.window_root(hwnd).and_then(|(target, root)| {
            root.SetScale(scale(k, k))?;
            Ok((target, root))
        });
        match connected {
            Ok((target, root)) => Ok(Self {
                hwnd,
                target: Some(target),
                root,
                screen: (
                    (monitor.right - monitor.left) as f32 / k,
                    (monitor.bottom - monitor.top) as f32 / k,
                ),
            }),
            Err(e) => {
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
                Err(e)
            }
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        // First disconnect the window from Composition, then close it.
        self.target = None;
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
