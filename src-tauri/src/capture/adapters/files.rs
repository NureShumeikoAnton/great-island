//! Files a capture leaves behind: Windows' known folders, a name that is still free,
//! showing a file in Explorer and opening it.

use crate::fault::{Cause, Failed};
use std::path::{Path, PathBuf};
use windows::{
    Win32::{
        System::Com::CoTaskMemFree,
        UI::{
            Shell::{KF_FLAG_CREATE, SHGetKnownFolderPath, ShellExecuteW},
            WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow, SW_SHOWNORMAL},
        },
    },
    core::{GUID, HSTRING, PCWSTR, w},
};

pub fn known_folder(id: &GUID) -> Option<PathBuf> {
    unsafe {
        let path = SHGetKnownFolderPath(id, KF_FLAG_CREATE, None).ok()?;
        let text = path.to_string().ok();
        CoTaskMemFree(Some(path.0 as *const _));
        text.map(PathBuf::from)
    }
}

/// "<stem>.<ext>" in `dir`; if taken, "<stem> (2).<ext>" and so on.
pub fn free_name(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}).{ext}")))
        .find(|p| !p.exists())
        .expect("a free name exists")
}

pub fn show_in_folder(path: &Path) -> Result<(), Failed> {
    // /select on a missing file makes Explorer open some unrelated folder; open the parent instead.
    if !path.exists() {
        let folder = path
            .parent()
            .filter(|p| p.is_dir())
            .ok_or_else(|| Failed::because(Cause::Missing, path.display()))?;
        let folder = HSTRING::from(folder);
        return shell_open(PCWSTR(folder.as_ptr()), PCWSTR::null());
    }
    let args = HSTRING::from(format!("/select,\"{}\"", path.display()));
    shell_open(w!("explorer.exe"), PCWSTR(args.as_ptr()))
}

/// In the program Windows opens such files with.
pub fn open(path: &Path) -> Result<(), Failed> {
    let file = HSTRING::from(path);
    shell_open(PCWSTR(file.as_ptr()), PCWSTR::null())
}

fn shell_open(file: PCWSTR, args: PCWSTR) -> Result<(), Failed> {
    let result = unsafe {
        // The opened window won't come to the front on its own: ours has just received a click and may allow it.
        let _ = AllowSetForegroundWindow(ASFW_ANY);
        ShellExecuteW(None, w!("open"), file, args, PCWSTR::null(), SW_SHOWNORMAL)
    };
    crate::platform::shell_result(result)
}
