//! Starting the other programs of an update: the update window (this exe in another mode) and the
//! installer; and sending the island a command from another process.

use crate::{fault::Failed, update::Command};
use std::{
    path::Path,
    process::{Child, Command as Process},
};
use windows::{
    Win32::{
        Foundation::{LPARAM, WPARAM},
        UI::WindowsAndMessaging::{FindWindowW, PostMessageW},
    },
    core::{PCWSTR, w},
};

/// The island's hidden window that takes messages from other processes (`island::window::SERVICE_CLASS`).
const SERVICE_CLASS: PCWSTR = w!("GreatIslandService");

/// The update window: this exe with `--update`.
pub fn open_window(lang: &str) -> Result<Child, Failed> {
    Ok(Process::new(std::env::current_exe()?)
        .arg("--update")
        .arg(format!("--lang={lang}"))
        .spawn()?)
}

/// The installer, silent, as an update: it keeps the data, closes what it must and starts the island again.
pub fn run_installer(installer: &Path) -> Result<(), Failed> {
    Process::new(installer)
        .args(["/S", "/UPDATE", "/R"])
        .spawn()?;
    Ok(())
}

pub fn island_runs() -> bool {
    unsafe { FindWindowW(SERVICE_CLASS, None).is_ok() }
}

/// False: there is no island to take it.
pub fn send(command: Command, message: u32) -> bool {
    unsafe {
        match FindWindowW(SERVICE_CLASS, None) {
            Ok(window) => {
                PostMessageW(Some(window), message, WPARAM(command as usize), LPARAM(0)).is_ok()
            }
            Err(_) => false,
        }
    }
}
