//! The Downloads folder watched on a thread of its own. Windows tells about every change in
//! the folder (ReadDirectoryChangesW), the rules (`Tracker`) decide, and what they decide goes
//! to the island with an arrived file's site and thumbnail, both read here: the shell can be
//! slow. While nothing is coming the thread sleeps in its wait.

use crate::{
    download::{Arrived, Change, Folder, Progress, Stat, Tracker, Update, source},
    fault::Failed,
    files::adapters::known_folder,
    media::{Icon, thumbnail},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread::JoinHandle,
    time::{SystemTime, UNIX_EPOCH},
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Storage::FileSystem::{
            CreateFileW, FILE_ACTION_ADDED, FILE_ACTION_MODIFIED, FILE_ACTION_REMOVED,
            FILE_ACTION_RENAMED_NEW_NAME, FILE_ACTION_RENAMED_OLD_NAME, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_FILE_NAME,
            FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE, FILE_NOTIFY_INFORMATION,
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
            ReadDirectoryChangesW,
        },
        System::{
            Com::{COINIT_MULTITHREADED, CoInitializeEx},
            IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
            Threading::{CreateEventW, INFINITE, ResetEvent, SetEvent, WaitForMultipleObjects},
        },
        UI::Shell::FOLDERID_Downloads,
    },
    core::{HSTRING, PCWSTR},
};

/// The card's thumbnail is 40 high: this is it at 200% and a little more.
const THUMB_PX: i32 = 96;
/// Room for a few hundred changes at once; more is an overflow the timer catches up with.
const BUFFER_WORDS: usize = 16 * 1024;

pub struct Arrival {
    pub path: PathBuf,
    pub name: String,
    pub bytes: u64,
    pub source: Option<String>,
    pub thumb: Option<Icon>,
}

pub enum Event {
    Coming(Progress),
    Arrived(Arrival),
    /// The watch ended by itself (the folder went away): nothing more comes.
    Stopped(Failed),
}

pub fn folder() -> Option<PathBuf> {
    known_folder(&FOLDERID_Downloads)
}

pub struct Watch {
    /// Set to end the thread.
    stop: HANDLE,
    thread: Option<JoinHandle<()>>,
    events: Receiver<Event>,
}

impl Watch {
    /// `notify` is called on the watch's thread after it sends events.
    pub fn start(dir: PathBuf, notify: impl Fn() + Send + 'static) -> Result<Self, Failed> {
        let handle = unsafe {
            CreateFileW(
                &HSTRING::from(dir.as_path()),
                FILE_LIST_DIRECTORY.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                None,
            )?
        };
        let stop = match unsafe { CreateEventW(None, true, false, PCWSTR::null()) } {
            Ok(stop) => stop,
            Err(e) => {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                return Err(e.into());
            }
        };
        let (tx, events) = mpsc::channel();
        // A HANDLE can't cross threads, a number can.
        let (handle_n, stop_n) = (handle.0 as isize, stop.0 as isize);
        let thread = std::thread::Builder::new()
            .name("downloads".into())
            .spawn(move || {
                let handle = HANDLE(handle_n as *mut _);
                run(&dir, handle, HANDLE(stop_n as *mut _), &tx, &notify);
                unsafe {
                    let _ = CloseHandle(handle);
                }
            });
        match thread {
            Ok(thread) => Ok(Self {
                stop,
                thread: Some(thread),
                events,
            }),
            Err(e) => {
                unsafe {
                    let _ = CloseHandle(handle);
                    let _ = CloseHandle(stop);
                }
                Err(e.into())
            }
        }
    }

    pub fn drain(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        unsafe {
            let _ = SetEvent(self.stop);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        unsafe {
            let _ = CloseHandle(self.stop);
        }
    }
}

fn run(dir: &Path, handle: HANDLE, stop: HANDLE, tx: &Sender<Event>, notify: &dyn Fn()) {
    // The thumbnails come from the shell.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let io = match unsafe { CreateEventW(None, true, false, PCWSTR::null()) } {
        Ok(io) => io,
        Err(e) => {
            let _ = tx.send(Event::Stopped(e.into()));
            return notify();
        }
    };
    let disk = Disk(dir);
    let mut tracker = Tracker::default();
    // ReadDirectoryChangesW wants a DWORD-aligned buffer.
    let mut buffer = vec![0u32; BUFFER_WORDS];
    // Lives across the loop: Windows writes into it while a read is pending.
    let mut overlapped: OVERLAPPED;
    let send = |updates: Vec<Update>| {
        if updates.is_empty() {
            return;
        }
        for update in updates {
            let event = match update {
                Update::Coming(progress) => Event::Coming(progress),
                Update::Arrived(arrived) => Event::Arrived(arrival(dir, arrived)),
            };
            let _ = tx.send(event);
        }
        notify();
    };
    let ended: Result<(), Failed> = 'watch: loop {
        overlapped = OVERLAPPED {
            hEvent: io,
            ..Default::default()
        };
        let armed = unsafe {
            let _ = ResetEvent(io);
            ReadDirectoryChangesW(
                handle,
                buffer.as_mut_ptr().cast(),
                (buffer.len() * 4) as u32,
                false,
                FILE_NOTIFY_CHANGE_FILE_NAME
                    | FILE_NOTIFY_CHANGE_SIZE
                    | FILE_NOTIFY_CHANGE_LAST_WRITE,
                None,
                Some(&mut overlapped),
                None,
            )
        };
        if let Err(e) = armed {
            break Err(e.into());
        }
        loop {
            let timeout = tracker
                .next_poll(now())
                .map_or(INFINITE, |ms| u32::try_from(ms).unwrap_or(INFINITE));
            let woke = unsafe { WaitForMultipleObjects(&[io, stop], false, timeout) };
            if woke == WAIT_OBJECT_0 {
                let mut bytes = 0;
                if let Err(e) =
                    unsafe { GetOverlappedResult(handle, &overlapped, &mut bytes, false) }
                {
                    break 'watch Err(e.into());
                }
                let mut updates = Vec::new();
                for change in changes(&buffer, bytes) {
                    updates.extend(tracker.change(change, now(), &disk));
                }
                send(updates);
                continue 'watch;
            }
            if woke == WAIT_TIMEOUT {
                send(tracker.poll(now(), &disk));
                continue;
            }
            // Asked to stop (or the wait failed): the pending read must end before its buffer goes.
            unsafe {
                let _ = CancelIoEx(handle, Some(&overlapped));
                let mut bytes = 0;
                let _ = GetOverlappedResult(handle, &overlapped, &mut bytes, true);
            }
            break 'watch Ok(());
        }
    };
    unsafe {
        let _ = CloseHandle(io);
    }
    if let Err(failed) = ended {
        let _ = tx.send(Event::Stopped(failed));
        notify();
    }
}

/// `bytes` = 0: more changed than the buffer holds, the list is lost.
fn changes(buffer: &[u32], bytes: u32) -> Vec<Change> {
    let mut out = Vec::new();
    if bytes == 0 {
        return out;
    }
    let base = buffer.as_ptr().cast::<u8>();
    let mut offset = 0usize;
    let mut old: Option<String> = None;
    loop {
        // SAFETY: Windows wrote `bytes` of FILE_NOTIFY_INFORMATION records into the buffer, each at a
        // DWORD-aligned offset, the name right after the fixed fields.
        let (action, name, next) = unsafe {
            let info = &*base.add(offset).cast::<FILE_NOTIFY_INFORMATION>();
            let chars = std::slice::from_raw_parts(
                std::ptr::addr_of!(info.FileName).cast::<u16>(),
                info.FileNameLength as usize / 2,
            );
            (
                info.Action,
                String::from_utf16_lossy(chars),
                info.NextEntryOffset,
            )
        };
        match action {
            FILE_ACTION_ADDED => out.push(Change::Added(name)),
            FILE_ACTION_REMOVED => out.push(Change::Removed(name)),
            FILE_ACTION_MODIFIED => out.push(Change::Modified(name)),
            // A name moved out of the folder has no new one: it is gone.
            FILE_ACTION_RENAMED_OLD_NAME => {
                if let Some(gone) = old.replace(name) {
                    out.push(Change::Removed(gone));
                }
            }
            FILE_ACTION_RENAMED_NEW_NAME => out.push(match old.take() {
                Some(from) => Change::Renamed { from, to: name },
                None => Change::Added(name),
            }),
            _ => {}
        }
        offset += next as usize;
        if next == 0 || offset >= bytes as usize {
            break;
        }
    }
    if let Some(gone) = old {
        out.push(Change::Removed(gone));
    }
    out
}

fn arrival(dir: &Path, arrived: Arrived) -> Arrival {
    let path = dir.join(&arrived.name);
    // Windows' mark on a file from the internet is a stream of its own next to the content.
    let mark = fs::read_to_string(format!("{}:Zone.Identifier", path.display())).ok();
    Arrival {
        source: mark.as_deref().and_then(source),
        thumb: thumbnail(&path, THUMB_PX),
        path,
        name: arrived.name,
        bytes: arrived.bytes,
    }
}

struct Disk<'a>(&'a Path);

impl Folder for Disk<'_> {
    fn stat(&self, name: &str) -> Option<Stat> {
        let meta = fs::metadata(self.0.join(name))
            .ok()
            .filter(fs::Metadata::is_file)?;
        Some(Stat {
            bytes: meta.len(),
            modified: meta.modified().map_or(0, unix_ms),
        })
    }
}

fn now() -> u64 {
    unix_ms(SystemTime::now())
}

fn unix_ms(at: SystemTime) -> u64 {
    at.duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
