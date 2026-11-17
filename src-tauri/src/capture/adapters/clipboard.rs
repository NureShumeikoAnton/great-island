//! Windows clipboard on behalf of a window: a picture as a DIB, which every program understands,
//! a file as Explorer puts it (CF_HDROP), and text.

use super::super::Image;
use crate::fault::{Cause, Failed};
use std::{path::Path, time::Duration};
use windows::{
    Win32::{
        Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND},
        Graphics::Gdi::{BI_RGB, BITMAPINFOHEADER},
        System::{
            DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
            Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
            Ole::{CF_DIB, CF_HDROP, CF_UNICODETEXT},
        },
        UI::Shell::DROPFILES,
    },
    core::{Error, Result},
};

pub struct SystemClipboard {
    owner: HWND,
}

impl SystemClipboard {
    pub fn new(owner: HWND) -> Self {
        Self { owner }
    }

    pub fn put_image(&mut self, image: &Image) -> std::result::Result<(), Failed> {
        let result =
            unsafe { dib(image).and_then(|memory| give(self.owner, CF_DIB.0 as u32, memory)) };
        result.map_err(busy)
    }

    pub fn put_text(&mut self, text: &str) -> std::result::Result<(), Failed> {
        let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
        let bytes = wide.len() * 2;
        let result = unsafe {
            fill(bytes, |target| {
                std::ptr::copy_nonoverlapping(wide.as_ptr().cast(), target, bytes);
            })
            .and_then(|memory| give(self.owner, CF_UNICODETEXT.0 as u32, memory))
        };
        result.map_err(busy)
    }

    pub fn put_file(&mut self, path: &Path) -> std::result::Result<(), Failed> {
        // A list of paths, each ending in a zero, and one more zero after the last.
        let wide: Vec<u16> = path
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .chain([0, 0])
            .collect();
        let head = size_of::<DROPFILES>();
        let bytes = wide.len() * 2;
        let files = DROPFILES {
            pFiles: head as u32,
            fWide: true.into(),
            ..Default::default()
        };
        let result = unsafe {
            fill(head + bytes, |target| {
                std::ptr::copy_nonoverlapping(&files as *const _ as *const u8, target, head);
                std::ptr::copy_nonoverlapping(wide.as_ptr().cast(), target.add(head), bytes);
            })
            .and_then(|memory| give(self.owner, u32::from(CF_HDROP.0), memory))
        };
        result.map_err(busy)
    }
}

/// Whatever the code says, a clipboard that can't be opened is held by another program.
fn busy(e: Error) -> Failed {
    Failed::because(Cause::Busy, e)
}

unsafe fn fill(size: usize, write: impl FnOnce(*mut u8)) -> Result<HGLOBAL> {
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, size)?;
        let target = GlobalLock(memory) as *mut u8;
        if target.is_null() {
            let _ = GlobalFree(Some(memory));
            return Err(Error::from_thread());
        }
        write(target);
        let _ = GlobalUnlock(memory);
        Ok(memory)
    }
}

unsafe fn dib(image: &Image) -> Result<HGLOBAL> {
    let header = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: image.width as i32,
        // Positive height = rows bottom to top: that is how every program reads a DIB.
        biHeight: image.height as i32,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        biSizeImage: image.pixels.len() as u32,
        ..Default::default()
    };
    let head = size_of::<BITMAPINFOHEADER>();
    unsafe {
        fill(head + image.pixels.len(), |target| {
            std::ptr::copy_nonoverlapping(&header as *const _ as *const u8, target, head);
            let row = image.width as usize * 4;
            for (i, line) in image.pixels.chunks_exact(row).rev().enumerate() {
                std::ptr::copy_nonoverlapping(line.as_ptr(), target.add(head + i * row), row);
            }
        })
    }
}

unsafe fn give(owner: HWND, format: u32, memory: HGLOBAL) -> Result<()> {
    unsafe {
        // The clipboard may be briefly held by another program: try several times.
        let mut opened = OpenClipboard(Some(owner));
        for _ in 0..5 {
            if opened.is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
            opened = OpenClipboard(Some(owner));
        }
        if let Err(e) = opened {
            let _ = GlobalFree(Some(memory));
            return Err(e);
        }
        let _ = EmptyClipboard();
        // After a successful hand-over the system owns the memory.
        let given = SetClipboardData(format, Some(HANDLE(memory.0)));
        let _ = CloseClipboard();
        if let Err(e) = given {
            let _ = GlobalFree(Some(memory));
            return Err(e);
        }
        Ok(())
    }
}
