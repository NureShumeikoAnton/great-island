//! The Recycle Bin, the way Explorer's Delete sends a file there.

use crate::fault::{Cause, Failed};
use std::path::Path;
use windows::{
    Win32::{
        System::Com::{CLSCTX_ALL, CoCreateInstance},
        UI::Shell::{
            FOF_ALLOWUNDO, FOF_NOERRORUI, FOFX_EARLYFAILURE, FOFX_RECYCLEONDELETE, FileOperation,
            IFileOperation, IShellItem, SHCreateItemFromParsingName,
        },
    },
    core::HSTRING,
};

/// false = the file stayed: Windows asked (a file too large for the bin goes for good, and
/// Windows says so) and the user kept it. Errors come back instead of Windows' own dialogs.
/// On a single-threaded apartment, as IFileOperation requires.
pub fn recycle(path: &Path) -> Result<bool, Failed> {
    // The video window's player lets the file go a moment after its window closed.
    let mut tries = 0;
    loop {
        match once(path) {
            Err(failed) if failed.cause == Cause::Busy && tries < 6 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            done => return done,
        }
    }
}

fn once(path: &Path) -> Result<bool, Failed> {
    unsafe {
        let operation: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
        operation.SetOperationFlags(
            FOF_ALLOWUNDO | FOF_NOERRORUI | FOFX_RECYCLEONDELETE | FOFX_EARLYFAILURE,
        )?;
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(path), None)?;
        operation.DeleteItem(&item, None)?;
        operation.PerformOperations()?;
        Ok(!operation.GetAnyOperationsAborted()?.as_bool())
    }
}
