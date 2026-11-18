//! Files through the Windows shell: known folders, opening a file in its program, showing
//! one or several files selected in Explorer.

use crate::fault::{Cause, Failed};
use std::path::{Path, PathBuf};
use windows::{
    Win32::{
        System::Com::CoTaskMemFree,
        UI::{
            Shell::{
                Common::ITEMIDLIST, ILCreateFromPathW, ILFindLastID, ILFree, KF_FLAG_CREATE,
                SHGetKnownFolderPath, SHOpenFolderAndSelectItems, ShellExecuteW,
            },
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

pub fn show_in_folder(path: &Path) -> Result<(), Failed> {
    // /select on a missing file makes Explorer open some unrelated folder; open the parent instead.
    if !path.exists() {
        return open_parent(path);
    }
    let args = HSTRING::from(format!("/select,\"{}\"", path.display()));
    shell_open(w!("explorer.exe"), PCWSTR(args.as_ptr()))
}

/// One Explorer window on their folder with all of them selected. They are expected in one
/// folder; the ones gone since are left out.
pub fn show_together(paths: &[PathBuf]) -> Result<(), Failed> {
    let present: Vec<&PathBuf> = paths.iter().filter(|p| p.exists()).collect();
    let first = match present.as_slice() {
        [] => {
            return paths
                .first()
                .map_or(Err(Failed::new(Cause::Missing)), |p| open_parent(p));
        }
        [only] => return show_in_folder(only),
        [first, ..] => first,
    };
    let folder = first
        .parent()
        .ok_or_else(|| Failed::because(Cause::Missing, first.display()))?;
    let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    let folder = Pidl::of(folder)?;
    let items = present
        .iter()
        .map(|p| Pidl::of(p))
        .collect::<Result<Vec<_>, _>>()?;
    // The items are named relative to the folder: the last part of each full name.
    let children: Vec<*const ITEMIDLIST> = items
        .iter()
        .map(|item| unsafe { ILFindLastID(item.0) } as *const _)
        .collect();
    unsafe { SHOpenFolderAndSelectItems(folder.0, Some(&children), 0)? };
    Ok(())
}

/// In the program Windows opens such files with.
pub fn open(path: &Path) -> Result<(), Failed> {
    let file = HSTRING::from(path);
    shell_open(PCWSTR(file.as_ptr()), PCWSTR::null())
}

fn open_parent(path: &Path) -> Result<(), Failed> {
    let folder = path
        .parent()
        .filter(|p| p.is_dir())
        .ok_or_else(|| Failed::because(Cause::Missing, path.display()))?;
    let folder = HSTRING::from(folder);
    shell_open(PCWSTR(folder.as_ptr()), PCWSTR::null())
}

fn shell_open(file: PCWSTR, args: PCWSTR) -> Result<(), Failed> {
    let result = unsafe {
        // The opened window won't come to the front on its own: ours has just received a click and may allow it.
        let _ = AllowSetForegroundWindow(ASFW_ANY);
        ShellExecuteW(None, w!("open"), file, args, PCWSTR::null(), SW_SHOWNORMAL)
    };
    crate::platform::shell_result(result)
}

/// A shell item id list, freed when dropped.
struct Pidl(*const ITEMIDLIST);

impl Pidl {
    fn of(path: &Path) -> Result<Self, Failed> {
        let pidl = unsafe { ILCreateFromPathW(&HSTRING::from(path)) };
        if pidl.is_null() {
            return Err(Failed::because(Cause::Missing, path.display()));
        }
        Ok(Self(pidl))
    }
}

impl Drop for Pidl {
    fn drop(&mut self) {
        unsafe { ILFree(Some(self.0)) };
    }
}
