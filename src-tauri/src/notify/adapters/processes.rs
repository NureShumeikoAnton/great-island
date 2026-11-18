//! Every process's processor time for the load notices (EnumProcesses, GetProcessTimes and
//! the exe path), and an app's name from its exe's version info.

use crate::{fault::Failed, notify::usage};
use std::{ffi::c_void, path::Path};
use windows::{
    Win32::{
        Foundation::{CloseHandle, FILETIME},
        Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW},
        System::{
            ProcessStatus::EnumProcesses,
            SystemInformation::{GetSystemInfo, GetSystemTimeAsFileTime, SYSTEM_INFO},
            Threading::{
                GetProcessTimes, OpenProcess, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
            },
        },
    },
    core::{HSTRING, PWSTR},
};

fn ticks(t: FILETIME) -> u64 {
    (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)
}

fn pids() -> Result<Vec<u32>, Failed> {
    let mut pids = vec![0u32; 1024];
    loop {
        let size = (pids.len() * 4) as u32;
        let mut used = 0u32;
        unsafe { EnumProcesses(pids.as_mut_ptr(), size, &mut used)? };
        // A full buffer may have cut the list short.
        if used < size {
            pids.truncate(used as usize / 4);
            return Ok(pids);
        }
        pids.resize(pids.len() * 2, 0);
    }
}

/// A process this user may not look into (a protected system one) is left out.
fn process(pid: u32) -> Option<usage::Process> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut times = [FILETIME::default(); 4];
    let [created, exited, kernel, user] = &mut times;
    let timed = unsafe { GetProcessTimes(handle, created, exited, kernel, user) }.is_ok();
    let mut path = [0u16; 1024];
    let mut len = path.len() as u32;
    let named = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(path.as_mut_ptr()),
            &mut len,
        )
    }
    .is_ok();
    unsafe {
        let _ = CloseHandle(handle);
    }
    (timed && named).then(|| usage::Process {
        pid,
        exe: String::from_utf16_lossy(&path[..len as usize]),
        cpu: ticks(times[2]) + ticks(times[3]),
    })
}

pub fn snapshot() -> Result<usage::Snapshot, Failed> {
    let processes = pids()?
        .into_iter()
        .filter(|&pid| pid != 0)
        .filter_map(process)
        .collect();
    let mut info = SYSTEM_INFO::default();
    unsafe { GetSystemInfo(&mut info) };
    Ok(usage::Snapshot {
        at: ticks(unsafe { GetSystemTimeAsFileTime() }),
        cores: info.dwNumberOfProcessors,
        processes,
    })
}

/// The "File description" of the exe ("Google Chrome"); `None` when it has none.
pub fn app_name(exe: &str) -> Option<String> {
    let path = HSTRING::from(exe);
    let size = unsafe { GetFileVersionInfoSizeW(&path, None) };
    if size == 0 {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    unsafe { GetFileVersionInfoW(&path, None, size, data.as_mut_ptr().cast()) }.ok()?;
    let query = |key: &str| -> Option<(*const c_void, u32)> {
        let mut ptr = std::ptr::null_mut();
        let mut len = 0u32;
        unsafe {
            VerQueryValueW(
                data.as_ptr().cast(),
                &HSTRING::from(key),
                &mut ptr,
                &mut len,
            )
        }
        .as_bool()
        .then_some((ptr.cast_const(), len))
    };
    // The first language the file has: pairs of u16, language and code page.
    let (ptr, len) = query("\\VarFileInfo\\Translation")?;
    if len < 4 {
        return None;
    }
    // SAFETY: VerQueryValueW points into `data` at `len` valid bytes, at least two u16 here.
    let (lang, page) = unsafe {
        let words = ptr.cast::<u16>();
        (*words, *words.add(1))
    };
    let (ptr, len) = query(&format!(
        "\\StringFileInfo\\{lang:04x}{page:04x}\\FileDescription"
    ))?;
    // SAFETY: a string value: `len` UTF-16 units including the trailing zero, inside `data`.
    let text = unsafe { std::slice::from_raw_parts(ptr.cast::<u16>(), len as usize) };
    let name = String::from_utf16_lossy(text)
        .trim_end_matches('\0')
        .trim()
        .to_string();
    (!name.is_empty()).then_some(name)
}

/// For an exe without a description: its file name without ".exe".
pub fn file_stem(exe: &str) -> String {
    Path::new(exe)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| exe.to_string())
}
