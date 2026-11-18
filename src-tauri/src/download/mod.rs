//! Downloads: which files in the Downloads folder are still coming and which have just
//! arrived. Pure rules, under tests: the folder's changes go in ([`Tracker`]), what to show
//! comes out ([`Update`]).
//!
//! - A browser writes into a partial file (`.crdownload`, `.part`, …) and renames it when done.
//!   While partial files exist something is coming; their total size is the progress.
//! - A file written straight (a messenger) has arrived once it is new, was written just now
//!   and stops changing for [`SETTLE_MS`]. A copied, moved or unpacked file keeps its old
//!   time: it is not news.
//! - Only the folder itself is watched: apps keep caches in folders inside it.
//!
//! The Windows side (the folder watch, a file's size and time, its thumbnail) is in `adapters/`.

pub mod adapters;

use crate::files::extension;

/// A file last written longer ago than this is a copy, not a download.
pub const FRESH_MS: u64 = 120_000;
/// A file written straight has arrived when it hasn't changed for this long.
pub const SETTLE_MS: u64 = 1_500;
/// While something is coming, its size is read this often.
pub const POLL_MS: u64 = 1_000;

/// Chromium browsers, Firefox, old Edge, Opera, Safari.
const PARTIAL: [&str; 5] = ["crdownload", "part", "partial", "opdownload", "download"];

pub fn is_partial(name: &str) -> bool {
    extension(name).is_some_and(|ext| PARTIAL.iter().any(|p| ext.eq_ignore_ascii_case(p)))
}

/// Never a download: Office's lock files, temp files, hidden ones, Explorer's desktop.ini.
fn is_noise(name: &str) -> bool {
    name.starts_with("~$")
        || name.starts_with('.')
        || name.eq_ignore_ascii_case("desktop.ini")
        || extension(name).is_some_and(|ext| ext.eq_ignore_ascii_case("tmp"))
}

/// The name a partial file will get. `None` while the browser hasn't chosen it yet
/// ("Unconfirmed 123456.crdownload").
pub fn final_name(partial: &str) -> Option<&str> {
    let (stem, _) = partial.rsplit_once('.')?;
    (extension(stem).is_some() && !stem.starts_with("Unconfirmed ")).then_some(stem)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Added(String),
    Removed(String),
    Modified(String),
    Renamed { from: String, to: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stat {
    pub bytes: u64,
    /// Unix ms.
    pub modified: u64,
}

/// The folder as the rules see it. `None`: gone, or not a file.
pub trait Folder {
    fn stat(&self, name: &str) -> Option<Stat>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arrived {
    pub name: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    /// 0: nothing is coming any more.
    pub files: usize,
    pub bytes: u64,
    /// When one file is coming and its name is known.
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Update {
    /// Said when it changes; after an arrival it comes second.
    Coming(Progress),
    Arrived(Arrived),
}

struct Settling {
    name: String,
    bytes: u64,
    changed: u64,
}

/// Times are Unix ms, the same clock as the files' own.
#[derive(Default)]
pub struct Tracker {
    coming: Vec<(String, u64)>,
    settling: Vec<Settling>,
    said: Progress,
}

impl Tracker {
    pub fn change(&mut self, change: Change, now: u64, folder: &impl Folder) -> Vec<Update> {
        let mut out = Vec::new();
        match change {
            Change::Added(name) => self.touch(name, true, now, folder),
            Change::Modified(name) => self.touch(name, false, now, folder),
            Change::Removed(name) => self.forget(&name),
            Change::Renamed { from, to } => {
                self.forget(&from);
                if is_partial(&from) && !is_partial(&to) && !is_noise(&to) {
                    // Firefox keeps an empty file under the final name until the end.
                    self.forget(&to);
                    if let Some(stat) = folder.stat(&to).filter(|s| s.bytes > 0) {
                        out.push(Update::Arrived(Arrived {
                            name: to,
                            bytes: stat.bytes,
                        }));
                    }
                } else {
                    self.touch(to, true, now, folder);
                }
            }
        }
        self.say_progress(&mut out);
        out
    }

    /// On the timer: the coming files' sizes, and the written ones that went quiet.
    pub fn poll(&mut self, now: u64, folder: &impl Folder) -> Vec<Update> {
        let mut out = Vec::new();
        self.coming
            .retain_mut(|(name, bytes)| match folder.stat(name) {
                Some(stat) => {
                    *bytes = stat.bytes;
                    true
                }
                None => false,
            });
        let coming = &self.coming;
        let partner = |name: &str| coming.iter().any(|(c, _)| final_name(c) == Some(name));
        self.settling.retain_mut(|file| {
            if now < file.changed + SETTLE_MS {
                return true;
            }
            let Some(stat) = folder.stat(&file.name) else {
                return false;
            };
            if stat.bytes != file.bytes {
                file.bytes = stat.bytes;
                file.changed = now;
                return true;
            }
            if stat.bytes > 0 && !partner(&file.name) {
                out.push(Update::Arrived(Arrived {
                    name: std::mem::take(&mut file.name),
                    bytes: stat.bytes,
                }));
            }
            false
        });
        self.say_progress(&mut out);
        out
    }

    /// How long until `poll` has something to do; `None`: nothing to wait for.
    pub fn next_poll(&self, now: u64) -> Option<u64> {
        let coming = (!self.coming.is_empty()).then_some(POLL_MS);
        let settling = self
            .settling
            .iter()
            .map(|f| (f.changed + SETTLE_MS).saturating_sub(now).max(1))
            .min();
        coming.into_iter().chain(settling).min()
    }

    fn touch(&mut self, name: String, added: bool, now: u64, folder: &impl Folder) {
        if is_noise(&name) {
            return;
        }
        if is_partial(&name) {
            let Some(stat) = folder.stat(&name) else {
                return;
            };
            match self.coming.iter_mut().find(|(n, _)| *n == name) {
                Some((_, bytes)) => *bytes = stat.bytes,
                None => self.coming.push((name, stat.bytes)),
            }
            return;
        }
        let Some(stat) = folder.stat(&name) else {
            return;
        };
        if let Some(file) = self.settling.iter_mut().find(|f| f.name == name) {
            file.bytes = stat.bytes;
            file.changed = now;
            return;
        }
        if added && stat.modified + FRESH_MS >= now {
            self.settling.push(Settling {
                name,
                bytes: stat.bytes,
                changed: now,
            });
        }
    }

    fn forget(&mut self, name: &str) {
        self.coming.retain(|(n, _)| n != name);
        self.settling.retain(|f| f.name != name);
    }

    fn say_progress(&mut self, out: &mut Vec<Update>) {
        let progress = Progress {
            files: self.coming.len(),
            bytes: self.coming.iter().map(|(_, b)| b).sum(),
            name: match self.coming.as_slice() {
                [(only, _)] => final_name(only).map(String::from),
                _ => None,
            },
        };
        if progress != self.said {
            self.said = progress.clone();
            out.push(Update::Coming(progress));
        }
    }
}

/// The site a file came from, by Windows' mark on it (`Zone.Identifier`): the page it was
/// downloaded from, else the address of the file itself. `None` without a web address.
pub fn source(mark: &str) -> Option<String> {
    let field = |key: &str| {
        mark.lines()
            .find_map(|line| line.trim().strip_prefix(key))
            .map(str::trim)
    };
    [field("ReferrerUrl="), field("HostUrl=")]
        .into_iter()
        .flatten()
        .find_map(host)
}

fn host(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = host.split(':').next()?.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    (!host.is_empty()).then(|| host.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::HashMap};

    const NOW: u64 = 1_800_000_000_000;

    #[derive(Default)]
    struct Disk(RefCell<HashMap<String, Stat>>);

    impl Disk {
        fn write(&self, name: &str, bytes: u64, modified: u64) {
            self.0
                .borrow_mut()
                .insert(name.into(), Stat { bytes, modified });
        }

        fn rename(&self, from: &str, to: &str) {
            let mut files = self.0.borrow_mut();
            let stat = files.remove(from).expect("the file exists");
            files.insert(to.into(), stat);
        }

        fn remove(&self, name: &str) {
            self.0.borrow_mut().remove(name);
        }
    }

    impl Folder for Disk {
        fn stat(&self, name: &str) -> Option<Stat> {
            self.0.borrow().get(name).copied()
        }
    }

    fn coming(files: usize, bytes: u64, name: Option<&str>) -> Update {
        Update::Coming(Progress {
            files,
            bytes,
            name: name.map(String::from),
        })
    }

    fn arrived(name: &str, bytes: u64) -> Update {
        Update::Arrived(Arrived {
            name: name.into(),
            bytes,
        })
    }

    fn renamed(from: &str, to: &str) -> Change {
        Change::Renamed {
            from: from.into(),
            to: to.into(),
        }
    }

    #[test]
    fn partial_files_and_their_final_names() {
        assert!(is_partial("report.pdf.crdownload"));
        assert!(is_partial("film.mkv.PART"));
        assert!(!is_partial("report.pdf"));
        assert_eq!(final_name("report.pdf.crdownload"), Some("report.pdf"));
        assert_eq!(final_name("Unconfirmed 120394.crdownload"), None);
        assert_eq!(final_name("archive.zip.part"), Some("archive.zip"));
    }

    #[test]
    fn a_chromium_download_comes_grows_and_arrives() {
        let disk = Disk::default();
        let mut t = Tracker::default();
        disk.write("Unconfirmed 1.crdownload", 0, NOW);
        let first = t.change(Change::Added("Unconfirmed 1.crdownload".into()), NOW, &disk);
        assert_eq!(first, [coming(1, 0, None)]);
        disk.rename("Unconfirmed 1.crdownload", "setup.exe.crdownload");
        let named = t.change(
            renamed("Unconfirmed 1.crdownload", "setup.exe.crdownload"),
            NOW,
            &disk,
        );
        assert_eq!(named, [coming(1, 0, Some("setup.exe"))]);
        assert_eq!(t.next_poll(NOW), Some(POLL_MS));

        disk.write("setup.exe.crdownload", 5_000_000, NOW + 1_000);
        assert_eq!(
            t.poll(NOW + 1_000, &disk),
            [coming(1, 5_000_000, Some("setup.exe"))]
        );
        assert!(t.poll(NOW + 2_000, &disk).is_empty(), "nothing new to say");

        disk.write("setup.exe.crdownload", 9_000_000, NOW + 3_000);
        disk.rename("setup.exe.crdownload", "setup.exe");
        let done = t.change(renamed("setup.exe.crdownload", "setup.exe"), NOW, &disk);
        assert_eq!(done, [arrived("setup.exe", 9_000_000), coming(0, 0, None)]);
        assert_eq!(t.next_poll(NOW), None);
    }

    #[test]
    fn firefox_keeps_an_empty_file_until_the_part_takes_its_place() {
        let disk = Disk::default();
        let mut t = Tracker::default();
        disk.write("film.mkv", 0, NOW);
        t.change(Change::Added("film.mkv".into()), NOW, &disk);
        disk.write("film.mkv.part", 100, NOW);
        t.change(Change::Added("film.mkv.part".into()), NOW, &disk);
        assert!(
            t.poll(NOW + SETTLE_MS, &disk).is_empty(),
            "the empty one is no download"
        );

        disk.remove("film.mkv");
        t.change(Change::Removed("film.mkv".into()), NOW + 2_000, &disk);
        disk.write("film.mkv.part", 700, NOW + 2_000);
        disk.rename("film.mkv.part", "film.mkv");
        let done = t.change(renamed("film.mkv.part", "film.mkv"), NOW + 2_000, &disk);
        assert_eq!(done, [arrived("film.mkv", 700), coming(0, 0, None)]);
    }

    #[test]
    fn a_file_written_straight_arrives_when_it_goes_quiet() {
        let disk = Disk::default();
        let mut t = Tracker::default();
        disk.write("photo.jpg", 10, NOW);
        assert!(
            t.change(Change::Added("photo.jpg".into()), NOW, &disk)
                .is_empty()
        );
        disk.write("photo.jpg", 900, NOW + 500);
        t.change(Change::Modified("photo.jpg".into()), NOW + 500, &disk);
        assert_eq!(t.next_poll(NOW + 500), Some(SETTLE_MS));
        assert!(t.poll(NOW + 1_600, &disk).is_empty(), "still being written");
        assert_eq!(t.poll(NOW + 2_000, &disk), [arrived("photo.jpg", 900)]);
        assert_eq!(t.next_poll(NOW + 2_000), None);
    }

    #[test]
    fn a_copied_or_unpacked_file_keeps_its_old_time_and_is_not_news() {
        let disk = Disk::default();
        let mut t = Tracker::default();
        disk.write("old.pdf", 300, NOW - FRESH_MS - 1);
        t.change(Change::Added("old.pdf".into()), NOW, &disk);
        assert!(t.poll(NOW + 5_000, &disk).is_empty());
        disk.write("renamed.pdf", 300, NOW - 3_600_000);
        t.change(renamed("before.pdf", "renamed.pdf"), NOW, &disk);
        assert_eq!(t.next_poll(NOW), None);
    }

    #[test]
    fn a_cancelled_download_just_stops_coming() {
        let disk = Disk::default();
        let mut t = Tracker::default();
        disk.write("a.zip.crdownload", 10, NOW);
        t.change(Change::Added("a.zip.crdownload".into()), NOW, &disk);
        disk.write("b.iso.crdownload", 30, NOW);
        let two = t.change(Change::Added("b.iso.crdownload".into()), NOW, &disk);
        assert_eq!(two, [coming(2, 40, None)]);
        disk.remove("a.zip.crdownload");
        let one = t.change(Change::Removed("a.zip.crdownload".into()), NOW, &disk);
        assert_eq!(one, [coming(1, 30, Some("b.iso"))]);
        disk.remove("b.iso.crdownload");
        assert_eq!(t.poll(NOW + 1_000, &disk), [coming(0, 0, None)]);
    }

    #[test]
    fn noise_is_never_a_download() {
        let disk = Disk::default();
        let mut t = Tracker::default();
        for name in ["~$report.docx", "desktop.ini", ".hidden", "~WRL0001.tmp"] {
            disk.write(name, 10, NOW);
            t.change(Change::Added(name.into()), NOW, &disk);
        }
        assert_eq!(t.next_poll(NOW), None);
    }

    #[test]
    fn the_source_is_the_page_else_the_file_address() {
        let chrome = "[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl=https://www.example.org/page?id=1\r\nHostUrl=https://cdn.example.net/file.zip\r\n";
        assert_eq!(source(chrome), Some("example.org".into()));
        let direct = "[ZoneTransfer]\nZoneId=3\nHostUrl=https://user@dl.example.com:8443/a.pdf\n";
        assert_eq!(source(direct), Some("dl.example.com".into()));
        assert_eq!(source("[ZoneTransfer]\nZoneId=3\n"), None);
        assert_eq!(
            source("[ZoneTransfer]\nZoneId=3\nHostUrl=about:internet\n"),
            None
        );
    }
}
