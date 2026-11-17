//! The journal: a small text file next to the settings. The release exe has no console,
//! so this is the only place where a failure leaves its details.
//!
//! Lines are rare (start, stop, a recording, a problem), never per frame.

use crate::{fault::Fault, persist};
use std::{
    fmt::Display,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::Mutex,
};
use windows::Win32::System::SystemInformation::GetLocalTime;

/// Past this size the file becomes `island.old.log` and a new one starts.
const LIMIT: u64 = 512 * 1024;

static WRITING: Mutex<()> = Mutex::new(());

pub fn path() -> PathBuf {
    persist::config_dir().join("island.log")
}

pub fn info(text: impl Display) {
    write("info", &text);
}

pub fn warn(text: impl Display) {
    write("warn", &text);
}

pub fn error(text: impl Display) {
    write("error", &text);
}

pub fn fault(fault: &Fault) {
    let mark = match fault.level() {
        crate::fault::Level::Warning => "warn",
        crate::fault::Level::Critical => "error",
    };
    let subject = fault.subject.as_deref().unwrap_or_default();
    write(
        mark,
        &format_args!(
            "{} {} {subject} {}",
            fault.code(),
            fault.title(),
            fault.detail
        ),
    );
}

fn write(mark: &str, text: &dyn Display) {
    // One entry — one line: a panic message comes in several.
    let text = text.to_string().replace(['\r', '\n'], " ");
    let line = format!("{} [{mark}] {}\n", stamp(), text.trim_end());
    if cfg!(debug_assertions) {
        eprint!("{line}");
    }
    if cfg!(test) {
        return;
    }
    // A poisoned lock means a panic mid-write: the journal still has to take the panic line.
    let _guard = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let path = path();
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if fs::metadata(&path).is_ok_and(|m| m.len() > LIMIT) {
        let _ = fs::rename(&path, path.with_extension("old.log"));
    }
    // Opened per line: the island and the settings window share the file.
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = file.write_all(line.as_bytes());
    }
}

fn stamp() -> String {
    let t = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    )
}
