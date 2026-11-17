//! Debug builds only (`npx tauri dev`), for tuning the level bars: a small window in the top-left corner with the player's raw
//! loudness from Windows (0–100 per channel), the gain that evens out the player's volume, and what
//! the bars get: the second bar's height, yellow, and what a hit adds on top, green. Updated with
//! every bars reading. Called from `island/mod.rs`, `island/player.rs` and
//! `island/view/player/bars.rs`, each call under `cfg(debug_assertions)`.

use crate::media::{Level, LevelSource};
use std::cell::Cell;
use windows::{
    Win32::{
        Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::{
            BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW,
            CreateSolidBrush, DeleteDC, DeleteObject, EndPaint, FW_NORMAL, FillRect, HDC,
            InvalidateRect, PAINTSTRUCT, SRCCOPY, SelectObject, SetBkMode, SetTextColor,
            TRANSPARENT, TextOutW,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::GetDpiForSystem,
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, GetClientRect, IDC_ARROW, LoadCursorW,
                RegisterClassW, WM_ERASEBKGND, WM_PAINT, WNDCLASSW, WS_CAPTION, WS_EX_NOACTIVATE,
                WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
            },
        },
    },
    core::w,
};

#[derive(Clone, Copy)]
struct Reading {
    level: Level,
    gain: f32,
    source: LevelSource,
}

thread_local! {
    static WINDOW: Cell<Option<HWND>> = const { Cell::new(None) };
    static READING: Cell<Option<Reading>> = const { Cell::new(None) };
    /// The second bar: (height without the pop, the pop's addition), 0 to 1.
    static BAR: Cell<(f32, f32)> = const { Cell::new((0.0, 0.0)) };
    /// The last hit seen and how many so far.
    static HITS: Cell<(Option<f64>, u32)> = const { Cell::new((None, 0)) };
}

pub(super) fn open() {
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            return;
        };
        let class = w!("GreatIslandLevels");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc);
        let k = GetDpiForSystem() as f32 / 96.0;
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class,
            w!("Player levels (debug)"),
            WS_POPUP | WS_CAPTION | WS_VISIBLE,
            0,
            0,
            (360.0 * k) as i32,
            (210.0 * k) as i32,
            None,
            None,
            Some(instance.into()),
            None,
        );
        if let Ok(hwnd) = hwnd {
            WINDOW.set(Some(hwnd));
        }
    }
}

pub(super) fn show_bar(base: f32, pop: f32, hit_at: Option<f64>) {
    BAR.set((base, pop));
    let (seen, count) = HITS.get();
    if hit_at.is_some() && hit_at != seen {
        HITS.set((hit_at, count + 1));
    }
}

pub(super) fn show(level: Level, gain: f32, source: LevelSource) {
    READING.set(Some(Reading {
        level,
        gain,
        source,
    }));
    if let Some(hwnd) = WINDOW.get() {
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }
}

extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // Everything is painted in WM_PAINT at once: no flicker.
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            unsafe { paint(hwnd) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

unsafe fn paint(hwnd: HWND) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let screen = BeginPaint(hwnd, &mut ps);
        let mut rect = RECT::default();
        let _ = GetClientRect(hwnd, &mut rect);
        let (w, h) = (rect.right, rect.bottom);
        let dc = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, w, h);
        let old_bitmap = SelectObject(dc, bitmap.into());
        let k = GetDpiForSystem() as f32 / 96.0;
        let px = |v: f32| (v * k) as i32;
        let font = CreateFontW(
            -px(14.0),
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            0,
            w!("Segoe UI"),
        );
        let old_font = SelectObject(dc, font.into());
        fill(dc, rect, rgb(32, 32, 32));
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, rgb(230, 230, 230));

        let reading = READING.get();
        let (left, right, gain, source) = match reading {
            Some(r) => (r.level.left, r.level.right, r.gain, r.source),
            None => (0.0, 0.0, 1.0, LevelSource::None),
        };
        let leveled = left.max(right) * gain;
        let (base, pop) = BAR.get();
        let rows = [
            ("Left", left, 0.0, rgb(80, 160, 255)),
            ("Right", right, 0.0, rgb(80, 160, 255)),
            ("Bars get", leveled, 0.0, rgb(160, 160, 160)),
            ("Bar 2", base, pop, rgb(240, 200, 40)),
        ];
        let green = rgb(70, 210, 90);
        let (bar_x, bar_w, row_h) = (px(80.0), w - px(80.0 + 64.0), px(32.0));
        for (i, (label, value, extra, color)) in rows.into_iter().enumerate() {
            let y = px(12.0) + i as i32 * row_h;
            text(dc, px(12.0), y + px(2.0), label);
            let track = RECT {
                left: bar_x,
                top: y,
                right: bar_x + bar_w,
                bottom: y + px(22.0),
            };
            fill(dc, track, rgb(60, 60, 60));
            let filled = RECT {
                right: bar_x + (bar_w as f32 * value.clamp(0.0, 1.0)) as i32,
                ..track
            };
            fill(dc, filled, color);
            if extra > 0.005 {
                let added = RECT {
                    left: filled.right,
                    right: bar_x + (bar_w as f32 * (value + extra).clamp(0.0, 1.0)) as i32,
                    ..track
                };
                fill(dc, added, green);
            }
            let shown = if extra > 0.005 {
                format!("{:.0}+{:.0}", value * 100.0, extra * 100.0)
            } else {
                format!("{:.0}", value * 100.0)
            };
            text(dc, bar_x + bar_w + px(8.0), y + px(2.0), &shown);
        }
        let db = |v: f32| 20.0 * v.max(1e-5).log10();
        let line_y = px(12.0) + rows.len() as i32 * row_h + px(4.0);
        let hits = HITS.get().1;
        text(
            dc,
            px(12.0),
            line_y,
            &format!(
                "gain ×{gain:.2}   raw {:.1} dB   {source:?}   hits {hits}",
                db(left.max(right))
            ),
        );
        if pop > 0.005 {
            SetTextColor(dc, green);
            text(dc, w - px(56.0), line_y, "POP");
        }

        let _ = BitBlt(screen, 0, 0, w, h, Some(dc), 0, 0, SRCCOPY);
        SelectObject(dc, old_font);
        SelectObject(dc, old_bitmap);
        let _ = DeleteObject(font.into());
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        let _ = EndPaint(hwnd, &ps);
    }
}

unsafe fn fill(dc: HDC, rect: RECT, color: COLORREF) {
    unsafe {
        let brush = CreateSolidBrush(color);
        FillRect(dc, &rect, brush);
        let _ = DeleteObject(brush.into());
    }
}

unsafe fn text(dc: HDC, x: i32, y: i32, s: &str) {
    let wide: Vec<u16> = s.encode_utf16().collect();
    unsafe {
        let _ = TextOutW(dc, x, y, &wide);
    }
}
