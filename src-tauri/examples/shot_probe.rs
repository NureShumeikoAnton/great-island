//! Probe for the screenshot system layer, without the island.
//!
//! `cargo run --release --example shot_probe -- <folder>` captures the primary monitor,
//! saves a PNG to the folder and prints how long it took. Then it briefly shows a red
//! square and checks that, with exclusion from capture enabled, it is absent from the shot.
//! Doesn't touch the clipboard.

use great_island_lib::{
    capture::{Area, Source, adapters::hide_from_capture},
    shot::{Gallery, adapters},
};
use std::time::{Duration, Instant};
use windows::{
    Win32::{
        Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::{CreateSolidBrush, MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetSystemMetrics,
            MSG, PM_REMOVE, PeekMessageW, RegisterClassW, SM_CXSCREEN, SM_CYSCREEN,
            SW_SHOWNOACTIVATE, ShowWindow, TranslateMessage, WNDCLASSW, WS_EX_NOACTIVATE,
            WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
        },
    },
    core::w,
};

fn primary() -> isize {
    unsafe { MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY) }.0 as isize
}

fn main() {
    let dir = std::env::args().nth(1).expect("укажите папку для PNG");
    let screen = Source::whole(
        primary(),
        unsafe { GetSystemMetrics(SM_CXSCREEN) } as u32,
        unsafe { GetSystemMetrics(SM_CYSCREEN) } as u32,
    );

    let t = Instant::now();
    let (image, stamp) = adapters::capture(screen).expect("снимок");
    println!(
        "снимок {}×{} за {:?}",
        image.width,
        image.height,
        t.elapsed()
    );
    let t = Instant::now();
    let preview = image.preview(1280);
    println!(
        "копия {}×{} за {:?}",
        preview.width,
        preview.height,
        t.elapsed()
    );
    let t = Instant::now();
    let path = adapters::Folder::new(&dir)
        .save(&image, stamp)
        .expect("PNG");
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    println!(
        "PNG {} ({} КБ) за {:?}",
        path.display(),
        size / 1024,
        t.elapsed()
    );

    let square = RECT {
        left: 100,
        top: 100,
        right: 300,
        bottom: 300,
    };
    let window = red_window(square);
    pump(Duration::from_millis(250));
    let visible = red_share(square);
    hide_from_capture(window, true);
    pump(Duration::from_millis(150));
    let hidden = red_share(square);
    unsafe {
        let _ = DestroyWindow(window);
    }
    println!(
        "красного на снимке: без скрытия {:.0}%, со скрытием {:.0}%",
        visible * 100.0,
        hidden * 100.0
    );
}

fn red_share(area: RECT) -> f32 {
    let source = Source {
        monitor: primary(),
        area: Area {
            x: area.left as u32,
            y: area.top as u32,
            width: (area.right - area.left) as u32,
            height: (area.bottom - area.top) as u32,
        },
    };
    let (image, _) = adapters::capture(source).expect("снимок квадрата");
    let red = image
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[2] > 200 && p[1] < 50 && p[0] < 50)
        .count();
    red as f32 / (image.width * image.height) as f32
}

fn red_window(r: RECT) -> HWND {
    unsafe {
        let instance = GetModuleHandleW(None).unwrap();
        let class = w!("ShotProbeRed");
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance.into(),
            lpszClassName: class,
            hbrBackground: CreateSolidBrush(COLORREF(0x0000FF)),
            ..Default::default()
        });
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class,
            w!("probe"),
            WS_POPUP,
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .unwrap();
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        hwnd
    }
}

fn pump(time: Duration) {
    let end = Instant::now() + time;
    while Instant::now() < end {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}
