//! Recording files: the folder, free disk space, the shelf for Ctrl+X. The clipboard is the
//! shared one from `capture`.

use super::super::{FileClipboard, Shelf};
use crate::{
    capture::adapters::{SystemClipboard, free_name, known_folder},
    fault::{Cause, Failed},
    lang::{self, Lang, tr, tr_for},
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use windows::{
    Win32::{Storage::FileSystem::GetDiskFreeSpaceExW, UI::Shell::FOLDERID_Videos},
    core::HSTRING,
};

/// Recordings stashed by Ctrl+X live this long; the clipboard has moved on by then.
const SHELF_LIFE: Duration = Duration::from_secs(24 * 60 * 60);

/// Videos\\Screen Recordings, named in the interface language. A folder made under the other
/// language is reused, so switching language doesn't split the recordings.
/// Deletes a recording for good, bypassing the recycle bin (like a deleted screenshot).
pub fn remove(path: &Path) -> Result<(), Failed> {
    // The video window's player lets the file go a moment after it stops: a sharing violation
    // right after closing the window is waited out.
    let mut tries = 0;
    loop {
        match std::fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(e) if e.raw_os_error() == Some(ERROR_SHARING_VIOLATION) && tries < 6 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => return Err(e.into()),
        }
    }
}

const ERROR_SHARING_VIOLATION: i32 = 32;

pub fn default_dir() -> PathBuf {
    let videos = known_folder(&FOLDERID_Videos).unwrap_or_else(|| PathBuf::from("."));
    let current = lang::current();
    std::iter::once(current)
        .chain(Lang::ALL.into_iter().filter(|lang| *lang != current))
        .map(|lang| videos.join(&*tr_for!(lang, "folder.recordings")))
        .find(|dir| dir.is_dir())
        .unwrap_or_else(|| videos.join(&*tr!("folder.recordings")))
}

/// Where to record: `custom` from the settings; empty — the default folder. A custom
/// folder that can't be used gives way to the default one, with the reason.
pub fn usable_dir(custom: &str) -> (PathBuf, Option<Failed>) {
    let custom = custom.trim();
    if custom.is_empty() {
        return (default_dir(), None);
    }
    match fs::create_dir_all(custom) {
        Ok(()) => (PathBuf::from(custom), None),
        Err(e) => (default_dir(), Some(e.into())),
    }
}

pub fn reserve(dir: &Path, stem: &str) -> Result<PathBuf, Failed> {
    fs::create_dir_all(dir)?;
    Ok(free_name(dir, stem, "mp4"))
}

pub fn free_space(dir: &Path) -> Option<u64> {
    let mut free = 0u64;
    unsafe { GetDiskFreeSpaceExW(&HSTRING::from(dir), Some(&mut free), None, None) }.ok()?;
    Some(free)
}

pub struct TempShelf {
    dir: PathBuf,
}

impl TempShelf {
    pub fn new() -> Self {
        Self {
            dir: std::env::temp_dir().join("Great Island").join("Recordings"),
        }
    }

    pub fn sweep(&self) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        let now = SystemTime::now();
        for entry in entries.flatten() {
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| now.duration_since(t).ok())
                .is_some_and(|age| age > SHELF_LIFE);
            if old {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

impl Shelf for TempShelf {
    fn stash(&mut self, path: &Path) -> Result<PathBuf, Failed> {
        fs::create_dir_all(&self.dir)?;
        let name = path
            .file_name()
            .ok_or_else(|| Failed::new(Cause::Missing))?;
        let target = self.dir.join(name);
        if fs::rename(path, &target).is_ok() {
            return Ok(target);
        }
        // Another disk: rename can't move across volumes.
        match fs::copy(path, &target).and_then(|_| fs::remove_file(path)) {
            Ok(()) => Ok(target),
            Err(e) => {
                let _ = fs::remove_file(&target);
                Err(e.into())
            }
        }
    }
}

impl FileClipboard for SystemClipboard {
    fn put_file(&mut self, path: &Path) -> Result<(), Failed> {
        SystemClipboard::put_file(self, path)
    }
}
