//! When the process goes down on its own: a panic in our code or an exception in the
//! system's (a driver, a codec). The build aborts on panic, so nothing can be caught —
//! but the journal gets the reason, a running recording gets its file closed, and the
//! island comes back by itself.

use crate::{journal, lang::tr, persist, platform, record};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use windows::Win32::System::Diagnostics::Debug::{EXCEPTION_POINTERS, SetUnhandledExceptionFilter};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Island,
    Settings,
}

/// Two crashes this close are a loop: the island stops restarting itself.
const LOOP_WINDOW: Duration = Duration::from_secs(60);
const EXCEPTION_CONTINUE_SEARCH: i32 = 0;

static DOWN: AtomicBool = AtomicBool::new(false);
static SETTINGS: AtomicBool = AtomicBool::new(false);

pub fn install(role: Role) {
    SETTINGS.store(role == Role::Settings, Ordering::Release);
    std::panic::set_hook(Box::new(|info| down(&format!("panic: {info}"))));
    unsafe {
        SetUnhandledExceptionFilter(Some(on_exception));
    }
}

unsafe extern "system" fn on_exception(info: *const EXCEPTION_POINTERS) -> i32 {
    let record = unsafe { info.as_ref().and_then(|i| i.ExceptionRecord.as_ref()) };
    let text = match record {
        Some(r) => format!(
            "exception 0x{:08X} at {:p}",
            r.ExceptionCode.0, r.ExceptionAddress
        ),
        None => "exception".into(),
    };
    down(&text);
    // Windows goes on to end the process as it would have without us.
    EXCEPTION_CONTINUE_SEARCH
}

fn down(text: &str) {
    // A second failure while handling the first must not start over.
    if DOWN.swap(true, Ordering::AcqRel) {
        return;
    }
    journal::error(text);
    let journal = journal::path().display().to_string();
    if SETTINGS.load(Ordering::Acquire) {
        platform::message_box(&tr!("crash.settings", journal = journal));
        return;
    }
    record::adapters::rescue();
    let looping = recent();
    let _ = fs::write(marker(), text);
    if looping {
        platform::message_box(&tr!("crash.loop", journal = journal));
    } else if let Err(e) = platform::relaunch(&["--restart"]) {
        journal::error(tr!("journal.crash_restart_failed", error = e));
    }
}

/// Written when the island goes down; `seen` once the next start has told the user.
fn marker() -> PathBuf {
    persist::config_dir().join("crash.txt")
}

fn seen_marker() -> PathBuf {
    persist::config_dir().join("crash.seen.txt")
}

fn recent() -> bool {
    [marker(), seen_marker()].iter().any(|path| {
        fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|at| at.elapsed().ok())
            .is_some_and(|age| age < LOOP_WINDOW)
    })
}

/// At start: did the island go down last time? Asked once per crash.
pub fn take() -> Option<String> {
    let text = fs::read_to_string(marker()).ok()?;
    // Renamed, not removed: its time still tells a crash loop from a single crash.
    let _ = fs::rename(marker(), seen_marker());
    Some(text)
}
